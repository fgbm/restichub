//! The typed outcome of a job from what its steps did (FR-AG-09, AC-RP-01, AC-HK-13).

use restichub_proto::agent::v1::{ErrorDetail, JobStatus, KeyValue};

use super::phases::{self, ScriptFailure};
use super::runner::StepError;

/// What the steps of one job did, as input of [`job_outcome`].
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
pub(super) struct Progress {
    pub(super) cancelled: bool,
    /// The exit code of the step that stopped the job, outside the missing-repository probe.
    pub(super) stopped_code: Option<i32>,
    pub(super) probe_missing: bool,
    pub(super) had_init: bool,
    pub(super) init_ran: bool,
    pub(super) init_succeeded: bool,
    pub(super) init_invalid: bool,
    /// Exit code 3 of a step: restic finished but could not read some files.
    pub(super) warned: bool,
    /// Set by the first failed script, so the job error names it (AC-HK-13).
    pub(super) script_failure: Option<ScriptFailure>,
    pub(super) limit: Option<StepError>,
    /// A restic step failed before the process started.
    pub(super) restic_not_started: bool,
}

/// Maps the step results to the typed outcome of a job (FR-AG-09, AC-RP-01).
///
/// The init chain: the agent runs init only after a `test_access` of the same repository exited with
/// code 10 (a missing repository). A successful init after that probe is a successful job. A
/// test_access that failed with any other code (1, 12, a timeout, access denied) stops the job,
/// and a standalone test_access with exit code 10 is the normal `NOT_INITIALIZED` result. An init
/// without a matching test_access is an invalid sequence and fails the job. Exit code 11 gives
/// `REPOSITORY_LOCKED` and exit code 3 gives `SUCCEEDED_WITH_WARNINGS`, so neither hides behind a
/// plain failure or a plain success.
pub(super) fn job_outcome(progress: &Progress) -> JobStatus {
    if progress.cancelled {
        return JobStatus::Cancelled;
    }
    if let Some(code) = progress.stopped_code {
        // restic exit code 12 is a wrong password or wrong credentials (AC-RP-01).
        return match code {
            12 => JobStatus::ConfigError,
            11 => JobStatus::RepositoryLocked,
            _ => JobStatus::Failed,
        };
    }
    if progress.init_invalid || progress.script_failure.is_some() {
        return JobStatus::Failed;
    }
    if progress.had_init && progress.init_ran {
        return if progress.init_succeeded {
            succeeded(progress)
        } else {
            JobStatus::Failed
        };
    }
    if !progress.had_init && progress.probe_missing {
        return JobStatus::NotInitialized;
    }
    succeeded(progress)
}

fn succeeded(progress: &Progress) -> JobStatus {
    if progress.warned {
        JobStatus::SucceededWithWarnings
    } else {
        JobStatus::Succeeded
    }
}

/// The stable code that explains a failed or cancelled outcome (NFR-DOC-04). It holds no secret.
pub(super) fn job_error(outcome: JobStatus, progress: &Progress) -> Option<ErrorDetail> {
    if outcome == JobStatus::Failed
        && progress.stopped_code.is_none()
        && let Some(failure) = &progress.script_failure
    {
        return Some(ErrorDetail {
            code: "hook_failed".to_owned(),
            params: vec![
                KeyValue {
                    key: "hook".to_owned(),
                    value: failure.script_name.clone(),
                },
                KeyValue {
                    key: "phase".to_owned(),
                    value: phases::phase_name(failure.phase).to_owned(),
                },
            ],
        });
    }
    let code = match outcome {
        JobStatus::Failed if progress.limit == Some(StepError::Stalled) => "stalled",
        JobStatus::Failed if progress.limit == Some(StepError::MaxDuration) => {
            "max_duration_exceeded"
        }
        JobStatus::ConfigError => "wrong_password",
        JobStatus::NotInitialized => "repository_missing",
        JobStatus::Failed if progress.restic_not_started => "step_not_started",
        JobStatus::Failed => "step_failed",
        JobStatus::Cancelled => "cancelled",
        JobStatus::RepositoryLocked => "repository_locked",
        JobStatus::SucceededWithWarnings => "succeeded_with_warnings",
        JobStatus::Succeeded | JobStatus::Unspecified => return None,
    };
    Some(ErrorDetail {
        code: code.to_owned(),
        params: Vec::new(),
    })
}

/// The error of a job that a restart interrupted before it finished. The agent runs no script of
/// such a job again (ADR-0037).
pub(super) fn interrupted_error() -> ErrorDetail {
    ErrorDetail {
        code: "interrupted".to_owned(),
        params: Vec::new(),
    }
}
