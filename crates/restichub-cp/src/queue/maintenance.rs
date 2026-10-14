//! The steps of a maintenance job: `forget`, `prune` and `check` (FR-RP-08, FR-PF-01, ADR-0034).
//!
//! The row of a maintenance job stores no command. The builder reads the profiles that use the
//! repository and the plans that run them, and it compiles each profile against the facts of the
//! agent of the plan.
//!
//! The retention of a profile may remove only the snapshots of that profile. Every `forget` step
//! therefore carries a filter of the host, the paths and the profile tag of its profile and agent,
//! and the grouping of the profile. A profile that two agents back up gives one step for each agent.

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

/// One `forget` step for each profile with a retention and each agent that backs it up.
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
        // The retention step needs the backup paths only, so a hook of the profile never blocks it.
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
            let mut backup_paths = None;
            let mut forget = None;
            for planned in compiled.steps {
                match planned.step.kind {
                    Some(step::Kind::Backup(backup)) => backup_paths = Some(backup.paths),
                    Some(step::Kind::Forget(step)) => forget = Some(step),
                    _ => {}
                }
            }
            let (Some(paths), Some(mut forget)) = (backup_paths, forget) else {
                continue;
            };
            forget.filter = Some(isolation_filter(profile.id, &agent.hostname, paths));
            if forget.group_by.is_empty() {
                forget.group_by = vec![ForgetGroupBy::Host as i32, ForgetGroupBy::Paths as i32];
            }
            steps.push(simple_step(
                &format!("forget-{}", steps.len() + 1),
                step::Kind::Forget(forget),
            ));
        }
    }
    Ok(steps)
}

/// The filter that limits a `forget` step to the snapshots of one profile on one host.
///
/// The profile tag is the exact key. The host and the paths narrow the step further and keep the
/// intent visible in the command line.
pub(super) fn isolation_filter(
    profile: ProfileId,
    hostname: &str,
    paths: Vec<String>,
) -> SnapshotsFilter {
    SnapshotsFilter {
        hosts: vec![hostname.to_owned()],
        tags: vec![profile_tag(profile)],
        paths,
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

/// The `check` step reads the subset of the first profile that sets one (FR-PF-01). A repository
/// without such a profile gets the check of the structure only.
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
            && !check.read_data_subset.is_empty()
        {
            read_data_subset = check.read_data_subset;
            break;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_rp_08_the_filter_names_the_host_the_paths_and_the_profile_tag() {
        let profile = ProfileId::new();
        let filter = isolation_filter(profile, "host-a", vec!["/srv".to_owned()]);
        assert_eq!(filter.hosts, ["host-a"]);
        assert_eq!(filter.paths, ["/srv"]);
        assert_eq!(filter.tags, [profile_tag(profile)]);
    }
}
