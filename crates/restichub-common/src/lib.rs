//! Shared types for ResticHub.

/// The package version with the build revision: `0.1.0+<short sha>`, or `0.1.0+dev` for a local
/// build. Version comparisons ignore the part after `+`.
pub const BUILD_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "+",
    env!("RESTICHUB_BUILD_REVISION")
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// restic exit code 0.
    Success,
    /// restic exit code 3: some source data could not be read.
    Warning,
    /// restic exit code 11: the repository is locked. The control plane retries later.
    Retry,
    /// restic exit codes 10 and 12: the repository does not exist or the password is wrong.
    ConfigError,
    /// restic exit code 130, or cancellation by the operator.
    Cancelled,
    /// All other exit codes.
    Failed,
}

impl JobStatus {
    /// Maps a restic exit code to a job status (FR-AG-09).
    ///
    /// An unknown exit code is a failure, as the restic documentation requires.
    pub fn from_restic_exit_code(code: i32) -> Self {
        match code {
            0 => Self::Success,
            3 => Self::Warning,
            11 => Self::Retry,
            10 | 12 => Self::ConfigError,
            130 => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JobStatus;

    #[test]
    fn ac_ag_05_exit_code_mapping() {
        let cases = [
            (0, JobStatus::Success),
            (1, JobStatus::Failed),
            (3, JobStatus::Warning),
            (10, JobStatus::ConfigError),
            (11, JobStatus::Retry),
            (12, JobStatus::ConfigError),
            (130, JobStatus::Cancelled),
            (99, JobStatus::Failed),
        ];
        for (code, expected) in cases {
            assert_eq!(
                JobStatus::from_restic_exit_code(code),
                expected,
                "exit code {code}"
            );
        }
    }
}
