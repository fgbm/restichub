//! The job builder: it turns a queued row into a signed-ready [`ValidatedJob`] (FR-PF-04, FR-PR-03,
//! FR-PR-05, FR-SC-05, ADR-0003).
//!
//! The row stores no command. The builder recompiles the stored profile of the plan against the
//! facts of the agent at send time, so a profile or an agent that changed since the schedule fires
//! gives a typed refusal and never a stale command. The job carries the secrets of its own
//! repository only (AC-PR-03). [`SecretReader`] writes the `secret_delivered` audit record in the
//! transaction of the caller (AC-RP-03); the dispatcher commits before it sends.

use restichub_compiler::{ValidatedJob, compile};
use restichub_db::jobs::Job;
use restichub_db::{
    AgentStatus, JobKind, JobResultCode, RepositoryId, TenantTx, agents as agents_repo,
    plans as plans_repo, profiles as profiles_repo, repositories as repo,
};
use restichub_proto::agent::v1::{Job as ProtoJob, Step, UnlockAllStep, UnlockStep, step};
use time::OffsetDateTime;

use crate::delivery::DeliveryError;
use crate::jobs::{repository_access, timestamp};
use crate::profiles::{compiler_profile_from_row, target_facts};
use crate::server::AppState;

/// The job timeout that the queue grants an agent (FR-AG-05).
const JOB_TIMEOUT_SECONDS: u32 = 12 * 3600;

/// The longest lifetime that the control plane signs. The agent rejects a lifetime above 24 h
/// (FR-PR-03).
const MAX_SIGNED_LIFETIME: time::Duration = time::Duration::hours(23);

/// A typed reason why the builder refuses a job. The code is stored as the result message and the
/// user interface translates it (NFR-DOC-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    AgentRevoked,
    PlanMissing,
    PlanChanged,
    ProfileMissing,
    RepositoryMissing,
    RepositoryChanged,
    AgentFactsInvalid,
    CompileFailed,
    StepMissing,
    UnsupportedKind,
    SecretUnavailable,
    JobInvalid,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::AgentRevoked => "agent_revoked",
            Self::PlanMissing => "plan_missing",
            Self::PlanChanged => "plan_changed",
            Self::ProfileMissing => "profile_missing",
            Self::RepositoryMissing => "repository_missing",
            Self::RepositoryChanged => "repository_changed",
            Self::AgentFactsInvalid => "agent_facts_invalid",
            Self::CompileFailed => "compile_failed",
            Self::StepMissing => "step_missing",
            Self::UnsupportedKind => "unsupported_kind",
            Self::SecretUnavailable => "secret_unavailable",
            Self::JobInvalid => "job_invalid",
        }
    }

    /// The typed result code of a job that the builder refuses.
    pub fn result_code(self) -> JobResultCode {
        match self {
            Self::AgentFactsInvalid
            | Self::CompileFailed
            | Self::StepMissing
            | Self::SecretUnavailable => JobResultCode::ConfigError,
            _ => JobResultCode::Failed,
        }
    }
}

/// Why the builder gave no job.
#[derive(Debug)]
pub enum BuildError {
    /// The job can never be sent as it is, so it fails with the typed reason.
    Refused(Refusal),
    /// A database or a key error. The job stays queued and the next pass tries again.
    Transient(String),
}

impl From<restichub_db::DbError> for BuildError {
    fn from(error: restichub_db::DbError) -> Self {
        Self::Transient(error.to_string())
    }
}

/// Recompiles the plan profile for the agent and assembles the job with the secrets of its own
/// repository (FR-PR-03).
pub async fn build(
    state: &AppState,
    tx: &mut TenantTx<'_>,
    job: &Job,
    now: OffsetDateTime,
) -> Result<ValidatedJob, BuildError> {
    let Some(agent) = agents_repo::get(tx, job.agent_id).await? else {
        return Err(BuildError::Refused(Refusal::AgentRevoked));
    };
    if agent.status != AgentStatus::Active {
        return Err(BuildError::Refused(Refusal::AgentRevoked));
    }
    let Some(repository_id) = job.repository_id else {
        return Err(BuildError::Refused(Refusal::RepositoryMissing));
    };
    let Some(repository) = repo::get(tx, repository_id).await? else {
        return Err(BuildError::Refused(Refusal::RepositoryMissing));
    };

    let steps = match job.kind {
        JobKind::Backup | JobKind::Forget | JobKind::Prune | JobKind::Check => {
            plan_steps(tx, job, &agent, repository_id, now).await?
        }
        JobKind::Unlock => vec![simple_step(
            "unlock",
            step::Kind::Unlock(UnlockStep {
                repository_id: repository_id.to_string(),
            }),
        )],
        JobKind::UnlockAll => vec![simple_step(
            "unlock-all",
            step::Kind::UnlockAll(UnlockAllStep {
                repository_id: repository_id.to_string(),
            }),
        )],
        JobKind::Init | JobKind::TestAccess | JobKind::Restore | JobKind::Other => {
            return Err(BuildError::Refused(Refusal::UnsupportedKind));
        }
    };

    let provider = state
        .kek()
        .ok_or_else(|| BuildError::Transient("the server holds no key provider".to_owned()))?;
    let access = repository_access(tx, provider, &repository, job.agent_id, job.id)
        .await
        .map_err(delivery_error)?;

    let mut expires_at = now + MAX_SIGNED_LIFETIME;
    if let Some(row_expiry) = job.expires_at {
        expires_at = expires_at.min(row_expiry);
    }
    let proto_job = ProtoJob {
        job_id: job.id.to_string(),
        tenant_id: tx.tenant().to_string(),
        agent_id: job.agent_id.to_string(),
        issued_at: Some(timestamp(now)),
        expires_at: Some(timestamp(expires_at)),
        timeout_seconds: JOB_TIMEOUT_SECONDS,
        repositories: vec![access],
        steps,
    };
    ValidatedJob::new(proto_job).map_err(|_| BuildError::Refused(Refusal::JobInvalid))
}

fn delivery_error(error: DeliveryError) -> BuildError {
    match error {
        DeliveryError::NotFound
        | DeliveryError::KeyMissing
        | DeliveryError::BadPurpose
        | DeliveryError::BadMaterial => BuildError::Refused(Refusal::SecretUnavailable),
        other => BuildError::Transient(other.to_string()),
    }
}

fn simple_step(step_id: &str, kind: step::Kind) -> Step {
    Step {
        step_id: step_id.to_owned(),
        timeout_seconds: 0,
        kind: Some(kind),
    }
}

/// Recompiles the stored profile of the plan for the agent and picks the steps of the job kind.
async fn plan_steps(
    tx: &mut TenantTx<'_>,
    job: &Job,
    agent: &restichub_db::agents::Agent,
    repository_id: RepositoryId,
    now: OffsetDateTime,
) -> Result<Vec<Step>, BuildError> {
    let Some(plan_id) = job.plan_id else {
        return Err(BuildError::Refused(Refusal::PlanMissing));
    };
    let Some(plan) = plans_repo::get(tx, plan_id).await? else {
        return Err(BuildError::Refused(Refusal::PlanMissing));
    };
    if plan.agent_id != job.agent_id {
        return Err(BuildError::Refused(Refusal::PlanChanged));
    }
    let Some(profile) = profiles_repo::get(tx, plan.profile_id).await? else {
        return Err(BuildError::Refused(Refusal::ProfileMissing));
    };
    if profile.repository_id != repository_id {
        return Err(BuildError::Refused(Refusal::RepositoryChanged));
    }
    let target =
        target_facts(agent, now).map_err(|_| BuildError::Refused(Refusal::AgentFactsInvalid))?;
    let compiler_profile = compiler_profile_from_row(&profile)
        .map_err(|_| BuildError::Refused(Refusal::CompileFailed))?;
    let compiled = compile(&compiler_profile, &target)
        .map_err(|_| BuildError::Refused(Refusal::CompileFailed))?;

    let mut steps: Vec<Step> = match job.kind {
        JobKind::Backup => compiled
            .steps
            .into_iter()
            .filter(|planned| !planned.maintenance)
            .map(|planned| planned.step)
            .collect(),
        kind => compiled
            .steps
            .into_iter()
            .filter(|planned| planned.maintenance && step_matches(kind, &planned.step))
            .map(|planned| planned.step)
            .collect(),
    };
    if steps.is_empty() {
        return Err(BuildError::Refused(Refusal::StepMissing));
    }
    for step in &mut steps {
        force_retry_lock(step);
    }
    Ok(steps)
}

fn step_matches(kind: JobKind, step: &Step) -> bool {
    matches!(
        (kind, step.kind.as_ref()),
        (JobKind::Forget, Some(step::Kind::Forget(_)))
            | (JobKind::Prune, Some(step::Kind::Prune(_)))
            | (JobKind::Check, Some(step::Kind::Check(_)))
    )
}

/// The control plane always sends `--retry-lock` (FR-SC-05).
fn force_retry_lock(step: &mut Step) {
    match step.kind.as_mut() {
        Some(step::Kind::Backup(backup)) => backup.retry_lock = true,
        Some(step::Kind::Forget(forget)) => forget.retry_lock = true,
        Some(step::Kind::Prune(prune)) => prune.retry_lock = true,
        Some(step::Kind::Check(check)) => check.retry_lock = true,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::BackupStep;

    use super::*;

    #[test]
    fn fr_sc_05_the_builder_forces_retry_lock() {
        let mut backup = simple_step(
            "backup",
            step::Kind::Backup(BackupStep {
                retry_lock: false,
                ..BackupStep::default()
            }),
        );
        force_retry_lock(&mut backup);
        assert!(matches!(
            backup.kind,
            Some(step::Kind::Backup(BackupStep {
                retry_lock: true,
                ..
            }))
        ));
    }

    #[test]
    fn a_refusal_has_a_stable_code() {
        assert_eq!(Refusal::CompileFailed.code(), "compile_failed");
        assert_eq!(
            Refusal::CompileFailed.result_code(),
            JobResultCode::ConfigError
        );
        assert_eq!(Refusal::AgentRevoked.result_code(), JobResultCode::Failed);
    }
}
