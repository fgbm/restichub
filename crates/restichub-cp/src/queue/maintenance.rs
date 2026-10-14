//! The steps of a maintenance job: `forget`, `prune` and `check` (FR-RP-08, FR-PF-01, ADR-0034).
//!
//! The row of a maintenance job stores no command. The builder reads the profiles that use the
//! repository and the plans that run them, and it compiles each profile against the facts of the
//! agent of the plan.
//!
//! The retention of a profile may remove only the snapshots of that profile. Each profile therefore
//! gives one `forget` step. It filters on the profile tag, joined with the filter of the profile,
//! and it groups by the grouping of the profile.

use std::collections::HashSet;

use restichub_compiler::compile;
use restichub_db::jobs::Job;
use restichub_db::{
    AgentId, AgentStatus, JobKind, ProfileId, RepositoryId, TenantTx, agents as agents_repo,
    plans as plans_repo, profiles as profiles_repo,
};
use restichub_proto::agent::v1::{
    CheckStep, ForgetGroupBy, PruneStep, SnapshotsFilter, Step, step,
};
use time::OffsetDateTime;

use super::build::{BuildError, Refusal, force_retry_lock, simple_step};
use crate::profiles::{compiler_profile_from_row, profile_tag, target_facts};

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
/// The step takes the facts of the first active agent that runs the profile and that the
/// `retention.filter.hosts` list of the profile allows. The host name of the agent compares with the
/// host name that restic records in its snapshots.
async fn forget_steps(
    tx: &mut TenantTx<'_>,
    job: &Job,
    repository_id: RepositoryId,
    now: OffsetDateTime,
) -> Result<Vec<Step>, BuildError> {
    let mut steps = Vec::new();
    for profile in profiles_repo::list_for_repository(tx, repository_id).await? {
        let mut compiler_profile = compiler_profile_from_row(&profile).map_err(|error| {
            tracing::warn!(job = %job.id, profile = %profile.id, cause = ?error, "the stored profile does not convert");
            BuildError::Refused(Refusal::CompileFailed)
        })?;
        if compiler_profile.retention.is_none() {
            continue;
        }
        // The retention step needs no hook, so a hook of the profile never blocks it.
        compiler_profile.hooks.clear();

        let mut seen: HashSet<AgentId> = HashSet::new();
        for plan in plans_repo::list_by_profile(tx, profile.id).await? {
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
            let compiled = match compile(&compiler_profile, &target) {
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
            let profile_filter = forget.filter.take().unwrap_or_default();
            if !host_allowed(&profile_filter, &agent.hostname) {
                continue;
            }
            forget.filter = Some(isolation_filter(profile.id, profile_filter));
            if forget.group_by.is_empty() {
                forget.group_by = vec![ForgetGroupBy::Host as i32, ForgetGroupBy::Paths as i32];
            }
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
fn host_allowed(filter: &SnapshotsFilter, hostname: &str) -> bool {
    filter.hosts.is_empty() || filter.hosts.iter().any(|host| host == hostname)
}

/// The filter of a `forget` step: the profile filter narrowed to the snapshots of one profile.
///
/// The profile tag is the exact key (FR-RP-08). The tags of the profile filter join it, so a
/// snapshot needs the profile tag and every tag of the filter. The step has no host filter of its
/// own, because the host name of an agent is editable. Each host and path set keeps its own policy
/// through the grouping.
pub(super) fn isolation_filter(profile: ProfileId, filter: SnapshotsFilter) -> SnapshotsFilter {
    let mut all_tags = vec![profile_tag(profile)];
    all_tags.extend(filter.tags);
    SnapshotsFilter {
        hosts: filter.hosts,
        tags: Vec::new(),
        paths: filter.paths,
        all_tags,
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

/// The `check` step reads the largest subset that a profile of the repository sets (FR-PF-01). A
/// repository without such a profile gets the check of the structure only. A tie goes to the oldest
/// profile.
async fn check_step(
    tx: &mut TenantTx<'_>,
    job: &Job,
    repository_id: RepositoryId,
) -> Result<Step, BuildError> {
    let mut read_data_subset = String::new();
    for profile in profiles_repo::list_for_repository(tx, repository_id).await? {
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
    fn fr_rp_08_the_filter_joins_the_profile_tag_with_the_profile_filter() {
        let profile = ProfileId::new();
        let filter = isolation_filter(
            profile,
            SnapshotsFilter {
                hosts: vec!["host-a".to_owned()],
                tags: vec!["daily".to_owned()],
                paths: vec!["/srv".to_owned()],
                all_tags: Vec::new(),
            },
        );
        assert_eq!(filter.hosts, ["host-a"]);
        assert_eq!(filter.paths, ["/srv"]);
        assert!(filter.tags.is_empty());
        assert_eq!(filter.all_tags, [profile_tag(profile), "daily".to_owned()]);
    }
}
