//! The job lifecycle diagram of the documentation names every job status, and only terminal
//! statuses have no way out.

use std::collections::BTreeSet;
use std::path::Path;

use restichub_db::enums::JobStatus;

fn every_status() -> [JobStatus; 7] {
    // A new variant breaks this match, so the list cannot fall behind the enum.
    let known = |status: JobStatus| match status {
        JobStatus::Queued
        | JobStatus::Sent
        | JobStatus::Running
        | JobStatus::Succeeded
        | JobStatus::Failed
        | JobStatus::Expired
        | JobStatus::DeadLetter => status,
    };
    [
        JobStatus::Queued,
        JobStatus::Sent,
        JobStatus::Running,
        JobStatus::Succeeded,
        JobStatus::Failed,
        JobStatus::Expired,
        JobStatus::DeadLetter,
    ]
    .map(known)
}

#[test]
fn the_job_lifecycle_diagram_matches_the_job_statuses() {
    let expected: BTreeSet<&str> = every_status().iter().map(|s| s.as_str()).collect();
    for lang in ["en", "ru"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../docs/src/diagrams/job-lifecycle.{lang}.mmd"));
        let source = std::fs::read_to_string(&path).expect("read the job lifecycle diagram");
        let states: BTreeSet<&str> = source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("state "))
            .filter_map(|line| line.rsplit(" as ").next())
            .collect();
        assert_eq!(states, expected, "the states of {}", path.display());
        for status in every_status() {
            let leaves = source
                .lines()
                .any(|line| line.trim().starts_with(&format!("{} -->", status.as_str())));
            assert_eq!(
                leaves,
                !status.is_terminal(),
                "{lang}: {status:?} has an outgoing edge only when it is not terminal"
            );
        }
    }
}
