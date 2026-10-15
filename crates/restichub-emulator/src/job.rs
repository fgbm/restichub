//! A sample job for the emulator tests and the test server.

use restichub_proto::agent::v1::{
    BackupStep, Job, LocalCredentials, RepositoryAccess, Step,
    repository_access::BackendCredentials, step,
};

use crate::identity::TENANT_ID;

/// Identifier of the one repository of a sample job.
pub const REPOSITORY_ID: &str = "repo-1";

/// Builds a small backup job for `agent_id` that passes `validate_job`.
#[must_use]
pub fn sample_job(agent_id: &str, job_id: &str) -> Job {
    let issued_at = crate::now_timestamp();
    let expires_at = prost_types::Timestamp {
        seconds: issued_at.seconds + 300,
        nanos: issued_at.nanos,
    };
    Job {
        job_id: job_id.to_owned(),
        tenant_id: TENANT_ID.to_owned(),
        agent_id: agent_id.to_owned(),
        issued_at: Some(issued_at),
        expires_at: Some(expires_at),
        timeout_seconds: 60,
        stall_timeout_seconds: 0,
        repositories: vec![RepositoryAccess {
            repository_id: REPOSITORY_ID.to_owned(),
            location: "/tmp/restichub-emulator-repo".to_owned(),
            password: "emulator-password".to_owned(),
            backend_credentials: Some(BackendCredentials::Local(LocalCredentials {})),
        }],
        steps: vec![Step {
            step_id: "step-1".to_owned(),
            kind: Some(step::Kind::Backup(BackupStep {
                repository_id: REPOSITORY_ID.to_owned(),
                paths: vec!["/srv".to_owned()],
                ..BackupStep::default()
            })),
            ..Step::default()
        }],
    }
}
