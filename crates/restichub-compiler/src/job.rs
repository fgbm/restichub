//! A job that passed the shared validation (FR-PR-10).

use restichub_proto::agent::v1::{Job, RepositoryAccess};
use restichub_proto::signing::Rejection;

/// A job that passed [`validate_job`](restichub_proto::validate::validate_job).
///
/// The argument builder accepts only this type, so an unvalidated job cannot
/// reach restic (FR-PR-10, ADR-0021).
#[derive(Debug, Clone)]
pub struct ValidatedJob {
    job: Job,
}

impl ValidatedJob {
    /// Validates a job before the compiler or the agent uses it (FR-PR-10).
    ///
    /// # Errors
    ///
    /// Returns the [`Rejection`] of the shared validation.
    pub fn new(job: Job) -> Result<Self, Rejection> {
        restichub_proto::validate::validate_job(&job)?;
        Ok(Self { job })
    }

    #[must_use]
    pub fn job(&self) -> &Job {
        &self.job
    }

    /// Finds the access record for one identifier.
    #[must_use]
    pub fn repository(&self, repository_id: &str) -> Option<&RepositoryAccess> {
        self.job
            .repositories
            .iter()
            .find(|repository| repository.repository_id == repository_id)
    }
}
