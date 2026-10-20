//! The typed outcome of a job from its step results (FR-AG-09, AC-RP-01).

use restichub_proto::agent::v1::{ErrorDetail, JobStatus};

/// Maps the step results to the typed outcome of a job (FR-AG-09, AC-RP-01).
///
/// The init chain: the agent runs init only after test_access exited with code 10 (a missing
/// repository). A successful init after that probe is a successful job. A test_access that failed
/// with any other code (1, 11, 12, a timeout, access denied) stops the job, and a standalone
/// test_access with exit code 10 is the normal `NOT_INITIALIZED` result.
pub(crate) fn job_outcome(
    cancelled: bool,
    stopped_code: Option<i32>,
    probe_missing: bool,
    had_init: bool,
    init_succeeded: bool,
) -> JobStatus {
    if cancelled {
        return JobStatus::Cancelled;
    }
    if let Some(code) = stopped_code {
        // restic exit code 12 is a wrong password or wrong credentials (AC-RP-01).
        return if code == 12 {
            JobStatus::ConfigError
        } else {
            JobStatus::Failed
        };
    }
    if probe_missing {
        if had_init {
            return if init_succeeded {
                JobStatus::Succeeded
            } else {
                JobStatus::Failed
            };
        }
        return JobStatus::NotInitialized;
    }
    JobStatus::Succeeded
}

/// The stable code that explains a failed or cancelled outcome (NFR-DOC-04). It holds no secret.
pub(crate) fn job_error(outcome: JobStatus) -> Option<ErrorDetail> {
    let code = match outcome {
        JobStatus::ConfigError => "wrong_password",
        JobStatus::NotInitialized => "repository_missing",
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

#[cfg(test)]
mod tests {
    use super::{job_error, job_outcome};
    use restichub_proto::agent::v1::JobStatus;

    /// AC-RP-01: the init chain runs init only after test_access exit code 10.
    #[test]
    fn ac_rp_01_maps_the_init_chain_to_a_job_outcome() {
        // A repository that exists: test_access exit 0, init skipped.
        assert_eq!(
            job_outcome(false, None, false, true, false),
            JobStatus::Succeeded
        );
        // A missing repository with a successful init.
        assert_eq!(
            job_outcome(false, None, true, true, true),
            JobStatus::Succeeded
        );
        // A standalone test_access on a missing repository.
        assert_eq!(
            job_outcome(false, None, true, false, false),
            JobStatus::NotInitialized
        );
        // A wrong password on an existing repository (exit code 12).
        assert_eq!(
            job_outcome(false, Some(12), false, true, false),
            JobStatus::ConfigError
        );
        // Any other failure (exit codes 1, 11, a timeout).
        assert_eq!(
            job_outcome(false, Some(1), false, true, false),
            JobStatus::Failed
        );
        assert_eq!(
            job_outcome(false, Some(11), false, true, false),
            JobStatus::Failed
        );
        // The control plane cancelled the job before any step finished.
        assert_eq!(
            job_outcome(true, None, false, true, false),
            JobStatus::Cancelled
        );
        // The error code that a wrong password reports.
        assert_eq!(
            job_error(JobStatus::ConfigError).map(|error| error.code),
            Some("wrong_password".to_owned())
        );
        assert_eq!(
            job_error(JobStatus::NotInitialized).map(|error| error.code),
            Some("repository_missing".to_owned())
        );
    }
}
