//! The steps of a browse job and of a restore: `snapshots`, `ls` and `restore` (FR-RS-01 to
//! FR-RS-04, ADR-0035).
//!
//! The row of the job stores no command. The builder reads the typed [`JobRequest`] that the API
//! wrote with the job and turns it into one step. The step carries the repository of the job only,
//! so the agent gets the secrets of that repository and no other (AC-PR-03).

use restichub_db::jobs::Job;
use restichub_db::restore::{self as restore_repo, JobRequest, OverwriteMode};
use restichub_db::{RepositoryId, TenantTx};
use restichub_proto::agent::v1::{
    LsStep, OverwriteMode as ProtoOverwriteMode, RestoreStep, SnapshotsStep, Step, step,
};

use super::build::{BuildError, Refusal, simple_step};

/// The timeout of a browse step. A person waits for it (AC-RS-02).
const BROWSE_TIMEOUT_SECONDS: u32 = 120;

pub(super) async fn steps(
    tx: &mut TenantTx<'_>,
    job: &Job,
    repository_id: RepositoryId,
) -> Result<Vec<Step>, BuildError> {
    let Some(request) = restore_repo::get_request(tx, job.id).await? else {
        return Err(BuildError::Refused(Refusal::RequestMissing));
    };
    let repository_id = repository_id.to_string();
    let step = match request {
        JobRequest::Snapshots(request) => Step {
            timeout_seconds: BROWSE_TIMEOUT_SECONDS,
            ..simple_step(
                "snapshots",
                step::Kind::Snapshots(SnapshotsStep {
                    hosts: request.hosts,
                    tags: request.tags,
                    paths: request.paths,
                    latest: 0,
                    repository_id,
                }),
            )
        },
        JobRequest::Ls(request) => Step {
            timeout_seconds: BROWSE_TIMEOUT_SECONDS,
            ..simple_step(
                "ls",
                step::Kind::Ls(LsStep {
                    snapshot_id: request.snapshot_id,
                    path: request.path,
                    recursive: false,
                    repository_id,
                    offset: request.offset,
                    limit: request.limit,
                }),
            )
        },
        JobRequest::Restore(request) => simple_step(
            "restore",
            step::Kind::Restore(RestoreStep {
                snapshot_id: request.snapshot_id,
                target: request.target,
                includes: request.includes,
                excludes: request.excludes,
                dry_run: request.dry_run,
                overwrite: proto_overwrite(request.overwrite) as i32,
                verify: request.verify,
                repository_id,
            }),
        ),
    };
    // A request of another kind than its job is a bug of the writer: the job never runs.
    let matches_kind = matches!(
        (job.kind, &step.kind),
        (
            restichub_db::JobKind::Snapshots,
            Some(step::Kind::Snapshots(_))
        ) | (restichub_db::JobKind::Ls, Some(step::Kind::Ls(_)))
            | (restichub_db::JobKind::Restore, Some(step::Kind::Restore(_)))
    );
    if !matches_kind {
        return Err(BuildError::Refused(Refusal::JobInvalid));
    }
    Ok(vec![step])
}

pub(crate) fn proto_overwrite(mode: OverwriteMode) -> ProtoOverwriteMode {
    match mode {
        OverwriteMode::Always => ProtoOverwriteMode::Always,
        OverwriteMode::IfChanged => ProtoOverwriteMode::IfChanged,
        OverwriteMode::IfNewer => ProtoOverwriteMode::IfNewer,
        OverwriteMode::Never => ProtoOverwriteMode::Never,
    }
}
