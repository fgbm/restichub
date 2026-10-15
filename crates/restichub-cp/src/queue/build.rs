//! The job builder: it turns a queued row into a signed-ready [`ValidatedJob`] (FR-PF-04, FR-PR-03,
//! FR-PR-05, FR-SC-05, ADR-0003).
//!
//! The row stores no command. The builder recompiles the stored profile of the plan against the
//! facts of the agent at the first send, so a profile or an agent that changed since the schedule
//! fires gives a typed refusal and never a stale command. A redelivery does not build: it sends the
//! stored signed bytes (ADR-0020). The job carries the secrets of its own
//! repository only (AC-PR-03). [`crate::delivery::SecretReader`] writes the `secret_delivered` audit record in the
//! transaction of the caller (AC-RP-03); the dispatcher commits before it sends.

use restichub_compiler::{JobLimits, ValidatedJob, compile};
use restichub_db::jobs::Job;
use restichub_db::{
    AgentStatus, JobKind, JobResultCode, RepositoryId, SecretId, TenantTx, agents as agents_repo,
    plans as plans_repo, profiles as profiles_repo, repositories as repo,
};
use restichub_proto::AgentCapability;
use restichub_proto::agent::v1::{
    Job as ProtoJob, Step, UnlockAllStep, UnlockStep, script_env, step,
};
use time::OffsetDateTime;

use crate::delivery::DeliveryError;
use crate::jobs::{delivered_secrets, repository_access, timestamp};
use crate::profiles::{compiler_profile_from_row, target_facts};
use crate::server::AppState;

/// The fixed job timeout that an agent without [`AgentCapability::JobLimitsV1`] still needs: it
/// applies its own default to a zero value (ADR-0039).
const LEGACY_JOB_TIMEOUT_SECONDS: u32 = 12 * 3600;

/// The maximum duration of a maintenance job: the default of a profile (ADR-0039).
const MAINTENANCE_MAX_DURATION_SECONDS: u32 = 48 * 3600;

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
    NotMaintenanceAgent,
    RequestMissing,
    AgentCapabilityMissing,
    ForgetWithoutTags,
    AgentFactsInvalid,
    CompileFailed,
    StepMissing,
    UnsupportedKind,
    SecretUnavailable,
    JobInvalid,
    /// The host of the agent did not turn on the trust mode for server scripts, or lacks the
    /// interpreter (AC-HK-07).
    ServerScriptsDisabled,
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
            Self::NotMaintenanceAgent => "not_maintenance_agent",
            Self::RequestMissing => "request_missing",
            Self::AgentCapabilityMissing => "agent_capability_missing",
            Self::ForgetWithoutTags => "forget_without_tags",
            Self::AgentFactsInvalid => "agent_facts_invalid",
            Self::CompileFailed => "compile_failed",
            Self::StepMissing => "step_missing",
            Self::UnsupportedKind => "unsupported_kind",
            Self::SecretUnavailable => "secret_unavailable",
            Self::JobInvalid => "job_invalid",
            Self::ServerScriptsDisabled => "server_scripts_disabled",
        }
    }

    /// The typed result code of a job that the builder refuses.
    pub fn result_code(self) -> JobResultCode {
        match self {
            Self::AgentFactsInvalid
            | Self::AgentCapabilityMissing
            | Self::ForgetWithoutTags
            | Self::CompileFailed
            | Self::StepMissing
            | Self::RequestMissing
            | Self::SecretUnavailable
            | Self::ServerScriptsDisabled => JobResultCode::ConfigError,
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

/// A job that is ready to sign, with the secrets that it carries.
pub struct Built {
    pub job: ValidatedJob,
    pub secrets: Vec<SecretId>,
}

/// Recompiles the plan profile for the agent and assembles the job with the secrets of its own
/// repository (FR-PR-03).
pub async fn build(
    state: &AppState,
    tx: &mut TenantTx<'_>,
    job: &Job,
    now: OffsetDateTime,
) -> Result<Built, BuildError> {
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

    let mut limits = JobLimits::default();
    let mut steps = match job.kind {
        JobKind::Backup => {
            let (steps, plan_limits) = plan_steps(tx, job, &agent, repository_id, now).await?;
            limits = plan_limits;
            steps
        }
        // Only the maintenance agent of the repository receives these steps (FR-RP-08, AC-RP-04).
        JobKind::Forget | JobKind::Prune | JobKind::Check => {
            if repository.maintenance_agent_id != Some(job.agent_id) {
                return Err(BuildError::Refused(Refusal::NotMaintenanceAgent));
            }
            limits.max_duration_seconds = MAINTENANCE_MAX_DURATION_SECONDS;
            super::maintenance::steps(tx, job, repository_id, now).await?
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
        JobKind::Snapshots | JobKind::Ls | JobKind::Restore => {
            super::restore::steps(tx, job, repository_id).await?
        }
        JobKind::Init | JobKind::TestAccess | JobKind::Other => {
            return Err(BuildError::Refused(Refusal::UnsupportedKind));
        }
    };

    // The capabilities of the live connection, never the stored row: a downgraded agent keeps its
    // identity (FR-PR-09). An agent that went offline since the snapshot keeps the job queued.
    let Some(capabilities) = state.agent_registry().capabilities(job.agent_id) else {
        return Err(BuildError::Transient("the agent is offline".to_owned()));
    };
    super::compat::fit_steps(&mut steps, &capabilities).map_err(BuildError::Refused)?;

    let provider = state
        .kek()
        .ok_or_else(|| BuildError::Transient("the server holds no key provider".to_owned()))?;
    let access = repository_access(tx, provider, &repository, job.agent_id, job.id)
        .await
        .map_err(delivery_error)?;
    let mut secrets = delivered_secrets(&repository);
    secrets.extend(
        resolve_hook_secrets(
            tx,
            provider,
            &mut steps,
            job.agent_id,
            job.id,
            repository_id,
        )
        .await
        .map_err(delivery_error)?,
    );

    record_script_run(tx, job, &steps).await?;

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
        timeout_seconds: match limits.max_duration_seconds {
            0 if !capabilities.contains(&AgentCapability::JobLimitsV1) => {
                LEGACY_JOB_TIMEOUT_SECONDS
            }
            seconds => seconds,
        },
        stall_timeout_seconds: if capabilities.contains(&AgentCapability::JobLimitsV1) {
            limits.stall_timeout_seconds
        } else {
            0
        },
        repositories: vec![access],
        steps,
    };
    let job = ValidatedJob::new(proto_job).map_err(|rejection| {
        tracing::warn!(job = %job.id, reason = ?rejection.reason, "the queue built a job that does not validate");
        BuildError::Refused(Refusal::JobInvalid)
    })?;
    Ok(Built { job, secrets })
}

/// Replaces the secret reference of each script variable with the value of the secret and returns
/// the secrets it read (FR-HK-09, ADR-0017).
///
/// The compiler leaves the identifier of the secret in the step. Only a secret of the purpose
/// `hook_secret` passes, and each read writes a `secret_delivered` record in the transaction of the
/// caller (AC-RP-03). A step that still holds an identifier never leaves the control plane.
async fn resolve_hook_secrets(
    tx: &mut TenantTx<'_>,
    provider: &dyn restichub_secrets::KekProvider,
    steps: &mut [Step],
    agent_id: restichub_db::AgentId,
    job_id: restichub_db::JobId,
    repository_id: RepositoryId,
) -> Result<Vec<SecretId>, DeliveryError> {
    let reader = crate::delivery::SecretReader::new(provider);
    let mut read = Vec::new();
    for step in steps {
        let mut references: Vec<&mut String> = Vec::new();
        if let Some(step::Kind::Script(script)) = step.kind.as_mut() {
            for variable in &mut script.env {
                if let Some(script_env::Value::SecretValue(reference)) = &mut variable.value {
                    references.push(reference);
                }
            }
        }
        for reference in references {
            let id = uuid::Uuid::parse_str(reference)
                .map(SecretId::from_uuid)
                .map_err(|error| {
                    tracing::warn!(%error, "a secret reference is not an identifier");
                    DeliveryError::NotFound
                })?;
            let value = reader
                .read_hook_secret_for_delivery(
                    tx,
                    id,
                    agent_id.as_uuid(),
                    job_id.as_uuid(),
                    repository_id.as_uuid(),
                )
                .await?;
            *reference = value.expose_secret().to_owned();
            read.push(id);
        }
    }
    Ok(read)
}

/// Writes the audit record of the scripts that leave with a job: the name, the phase and the
/// SHA-256 hash of each body, never the body or a value (FR-HK-12). The record is in the
/// transaction of the dispatch, so a job never leaves without it.
async fn record_script_run(
    tx: &mut TenantTx<'_>,
    job: &Job,
    steps: &[Step],
) -> Result<(), BuildError> {
    let scripts: Vec<serde_json::Value> = steps
        .iter()
        .filter_map(|step| match step.kind.as_ref() {
            Some(step::Kind::Script(script)) => Some(serde_json::json!({
                "step_id": step.step_id,
                "name": script.name,
                "phase": script.phase,
                "sha256": crate::scripts::body_hash(&script.body),
            })),
            _ => None,
        })
        .collect();
    if scripts.is_empty() {
        return Ok(());
    }
    let mut event =
        restichub_db::audit::AuditEvent::new(restichub_db::audit::AuditAction::ScriptRun, "job");
    event.object_id = Some(job.id.as_uuid());
    event.diff = Some(restichub_db::audit::diff(
        None,
        Some(&serde_json::json!({
            "job_id": job.id.to_string(),
            "agent_id": job.agent_id.to_string(),
            "scripts": scripts,
        })),
        &[],
    ));
    restichub_db::audit::insert(tx, &event).await?;
    Ok(())
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

pub(super) fn simple_step(step_id: &str, kind: step::Kind) -> Step {
    Step {
        step_id: step_id.to_owned(),
        timeout_seconds: 0,
        kind: Some(kind),
    }
}

/// Recompiles the stored profile of the plan for the agent and picks its backup steps.
async fn plan_steps(
    tx: &mut TenantTx<'_>,
    job: &Job,
    agent: &restichub_db::agents::Agent,
    repository_id: RepositoryId,
    now: OffsetDateTime,
) -> Result<(Vec<Step>, JobLimits), BuildError> {
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
    let target = target_facts(agent, now).map_err(|error| {
        tracing::warn!(job = %job.id, cause = ?error, "the agent facts do not give a compile target");
        BuildError::Refused(Refusal::AgentFactsInvalid)
    })?;
    let compiler_profile = compiler_profile_from_row(&profile).map_err(|error| {
        tracing::warn!(job = %job.id, cause = ?error, "the stored profile does not convert");
        BuildError::Refused(Refusal::CompileFailed)
    })?;
    let compiled = compile(&compiler_profile, &target).map_err(|error| {
        tracing::warn!(job = %job.id, cause = ?error, "the stored profile does not compile");
        if error.is_script() {
            BuildError::Refused(Refusal::ServerScriptsDisabled)
        } else {
            BuildError::Refused(Refusal::CompileFailed)
        }
    })?;

    let limits = compiled.limits;
    let mut steps: Vec<Step> = compiled
        .steps
        .into_iter()
        .filter(|planned| !planned.maintenance)
        .map(|planned| planned.step)
        .collect();
    if steps.is_empty() {
        return Err(BuildError::Refused(Refusal::StepMissing));
    }
    for step in &mut steps {
        force_retry_lock(step);
    }
    Ok((steps, limits))
}

/// The control plane always sends `--retry-lock` (FR-SC-05).
pub(super) fn force_retry_lock(step: &mut Step) {
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
