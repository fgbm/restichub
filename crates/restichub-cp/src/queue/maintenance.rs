//! The steps of a maintenance job: `forget`, `prune` and `check` (FR-RP-08, FR-PF-01, ADR-0034).
//!
//! The row of a maintenance job stores no command. The builder reads the plans that back up into
//! the repository and their profiles, and it compiles each profile for the repository against the
//! facts of the agent of the plan.
//!
//! The retention of a profile may remove only the snapshots of that profile. Each profile therefore
//! gives one `forget` step. It filters on the profile tag, joined with the filter of the profile,
//! and it groups by the grouping of the profile, with the host always added (ADR-0034).

use std::collections::HashSet;

use restichub_compiler::compile;
use restichub_db::jobs::Job;
use restichub_db::{
    AgentId, AgentStatus, JobKind, RepositoryId, TenantTx, agents as agents_repo,
    plans as plans_repo, profiles as profiles_repo,
};
use restichub_proto::agent::v1::{
    CheckStep, ForgetGroupBy, PruneStep, SnapshotsFilter, Step, step,
};
use time::OffsetDateTime;

use super::build::{BuildError, Refusal, force_retry_lock, simple_step};
use crate::profiles::{compiler_profile_from_row, target_facts};

/// Builds the steps of a `forget`, `prune` or `check` job for the repository.
///
/// A `forget` job runs the `forget` step of every profile and agent pair, then one `prune` step.
pub(super) async fn steps(
    tx: &mut TenantTx<'_>,
    job: &Job,
    repository_id: RepositoryId,
    now: OffsetDateTime,
) -> Result<Vec<Step>, BuildError> {
    let mut steps = match job.kind {
        JobKind::Forget => {
            let mut steps = forget_steps(tx, job, repository_id, now).await?;
            if !steps.is_empty() {
                steps.push(prune_step(repository_id));
            }
            steps
        }
        JobKind::Prune => vec![prune_step(repository_id)],
        JobKind::Check => vec![check_step(tx, job, repository_id).await?],
        _ => return Err(BuildError::Refused(Refusal::UnsupportedKind)),
    };
    if steps.is_empty() {
        return Err(BuildError::Refused(Refusal::StepMissing));
    }
    for step in &mut steps {
        force_retry_lock(step);
    }
    Ok(steps)
}

/// Gives the retention of every profile that has one, as a single step for each profile.
///
/// The step takes the facts of the first active agent that runs the profile into this repository
/// and that the `retention.filter.hosts` list of the profile allows. The host name of the agent
/// compares with the host name that restic records in its snapshots.
async fn forget_steps(
    tx: &mut TenantTx<'_>,
    job: &Job,
    repository_id: RepositoryId,
    now: OffsetDateTime,
) -> Result<Vec<Step>, BuildError> {
    let mut steps = Vec::new();
    for profile_id in plans_repo::profiles_of_repository(tx, repository_id).await? {
        let Some(profile) = profiles_repo::get(tx, profile_id).await? else {
            continue;
        };
        let compiler_profile = compiler_profile_from_row(&profile).map_err(|error| {
            tracing::warn!(job = %job.id, profile = %profile.id, cause = ?error, "the stored profile does not convert");
            BuildError::Refused(Refusal::CompileFailed)
        })?;
        if compiler_profile.retention.is_none() {
            continue;
        }

        let mut seen: HashSet<AgentId> = HashSet::new();
        for plan in
            plans_repo::list_by_profile_and_repository(tx, profile.id, repository_id).await?
        {
            if !seen.insert(plan.agent_id) {
                continue;
            }
            let Some(agent) = agents_repo::get(tx, plan.agent_id).await? else {
                continue;
            };
            if agent.status != AgentStatus::Active {
                continue;
            }
            let target = match target_facts(&agent, now) {
                Ok(target) => target,
                Err(error) => {
                    tracing::warn!(job = %job.id, agent = %agent.id, cause = ?error, "the agent facts give no compile target");
                    continue;
                }
            };
            let compiled = match compile(
                &compiler_profile,
                Some(&repository_id.to_string()),
                &target,
            ) {
                Ok(compiled) => compiled,
                Err(error) => {
                    tracing::warn!(job = %job.id, profile = %profile.id, cause = ?error, "the profile does not compile for the agent");
                    continue;
                }
            };
            let Some(mut forget) =
                compiled
                    .steps
                    .into_iter()
                    .find_map(|planned| match planned.step.kind {
                        Some(step::Kind::Forget(step)) => Some(step),
                        _ => None,
                    })
            else {
                continue;
            };
            if !host_allowed(forget.filter.as_ref(), &agent.hostname) {
                continue;
            }
            ensure_host_group(&mut forget.group_by);
            steps.push(simple_step(
                &format!("forget-{}", steps.len() + 1),
                step::Kind::Forget(forget),
            ));
            break;
        }
    }
    Ok(steps)
}

/// True when the host list of the profile filter is empty or names the host.
fn host_allowed(filter: Option<&SnapshotsFilter>, hostname: &str) -> bool {
    filter.is_none_or(|filter| {
        filter.hosts.is_empty() || filter.hosts.iter().any(|host| host == hostname)
    })
}

/// Groups the snapshots by host. The profile may run on several agents, and the retention of one
/// agent never counts the snapshots of another agent (ADR-0034). A grouping that holds no host gets
/// one; an empty grouping gets the restic default.
fn ensure_host_group(group_by: &mut Vec<i32>) {
    if group_by.is_empty() {
        *group_by = vec![ForgetGroupBy::Host as i32, ForgetGroupBy::Paths as i32];
    } else if !group_by.contains(&(ForgetGroupBy::Host as i32)) {
        group_by.insert(0, ForgetGroupBy::Host as i32);
    }
}

fn prune_step(repository_id: RepositoryId) -> Step {
    simple_step(
        "prune",
        step::Kind::Prune(PruneStep {
            repository_id: repository_id.to_string(),
            ..PruneStep::default()
        }),
    )
}

/// The `check` step reads the largest subset that a profile of the plans of the repository sets
/// (FR-PF-01). A repository without such a profile gets the check of the structure only. A tie goes
/// to the oldest profile.
async fn check_step(
    tx: &mut TenantTx<'_>,
    job: &Job,
    repository_id: RepositoryId,
) -> Result<Step, BuildError> {
    let mut read_data_subset = String::new();
    for profile_id in plans_repo::profiles_of_repository(tx, repository_id).await? {
        let Some(profile) = profiles_repo::get(tx, profile_id).await? else {
            continue;
        };
        let compiler_profile = compiler_profile_from_row(&profile).map_err(|error| {
            tracing::warn!(job = %job.id, profile = %profile.id, cause = ?error, "the stored profile does not convert");
            BuildError::Refused(Refusal::CompileFailed)
        })?;
        if let Some(check) = compiler_profile.check
            && subset_share(&check.read_data_subset) > subset_share(&read_data_subset)
        {
            read_data_subset = check.read_data_subset;
        }
    }
    Ok(simple_step(
        "check",
        step::Kind::Check(CheckStep {
            repository_id: repository_id.to_string(),
            read_data_subset,
            ..CheckStep::default()
        }),
    ))
}

/// The share of the data that a `read_data_subset` value reads, or none for an empty value.
fn subset_share(value: &str) -> Option<f64> {
    if value.is_empty() {
        return None;
    }
    let share = if let Some(percent) = value.strip_suffix('%') {
        percent.parse::<f64>().ok().map(|percent| percent / 100.0)
    } else if let Some((n, t)) = value.split_once('/') {
        match (n.parse::<f64>(), t.parse::<f64>()) {
            (Ok(n), Ok(t)) if t > 0.0 => Some(n / t),
            _ => None,
        }
    } else {
        None
    };
    Some(share.unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_pf_01_the_subset_share_orders_the_values() {
        assert_eq!(subset_share(""), None);
        assert!(subset_share("") < subset_share("0%"));
        assert!(subset_share("1/10") < subset_share("25%"));
        assert!(subset_share("25%") < subset_share("5/5"));
    }

    #[test]
    fn fr_rp_08_the_grouping_always_holds_the_host() {
        let host = ForgetGroupBy::Host as i32;
        let paths = ForgetGroupBy::Paths as i32;
        let tags = ForgetGroupBy::Tags as i32;
        let mut empty = Vec::new();
        ensure_host_group(&mut empty);
        assert_eq!(empty, [host, paths]);
        let mut without_host = vec![paths];
        ensure_host_group(&mut without_host);
        assert_eq!(without_host, [host, paths]);
        let mut with_host = vec![tags, host];
        ensure_host_group(&mut with_host);
        assert_eq!(with_host, [tags, host]);
    }
}
