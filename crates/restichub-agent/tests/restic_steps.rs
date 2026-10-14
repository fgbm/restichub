//! The backup and maintenance steps against the real restic and a local repository (AC-AG-05,
//! FR-AG-08, FR-PR-02).
//!
//! The test skips with a message when no restic runs on the machine, unless
//! `RESTICHUB_REQUIRE_RESTIC` asks it to fail. It needs no database.

#![allow(clippy::expect_used)]

mod common;

use common::*;
use restichub_proto::agent::v1::{
    BackupStep, JobStatus, ProcessPriority, StepStatus, step, step_result::Output,
};

fn directory(name: &str) -> std::path::PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

/// One job runs the whole life of a repository: init, two backups, snapshots, check, forget,
/// prune and stats. Every step must succeed with the typed output of restic.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_05_the_real_restic_runs_the_backup_and_maintenance_steps() {
    let Some(restic) = real_restic_or_skip() else {
        return;
    };
    let agent_dir = directory("real-steps-agent");
    let repository = temp_config_dir("real-steps-repo");
    let data = directory("real-steps-data");
    std::fs::write(data.join("a.txt"), "first file").expect("cannot write the file");
    std::fs::write(data.join("b.txt"), "second file").expect("cannot write the file");
    let data_path = data.to_string_lossy().into_owned();

    let jobs = real_jobs(&agent_dir, &restic);
    let access = access("repo-1", &repository, "CANARY-real-steps");
    let first = job(
        "job-init",
        vec![access.clone()],
        vec![
            test_access("repo-1"),
            init("repo-1"),
            backup_step("repo-1", &[&data_path]),
        ],
    );
    let result = run(&jobs, &first).await;
    assert_eq!(result.result().outcome, JobStatus::Succeeded as i32);
    // The probe exits with code 10 for the missing repository, then init and backup succeed.
    assert_eq!(result.step("test-access").exit_code, 10);
    assert_eq!(result.step("init").status, StepStatus::Success as i32);
    let Some(Output::BackupSummary(summary)) = result.step("backup").output.clone() else {
        panic!("the backup must give a typed summary");
    };
    assert_eq!(summary.files_new, 2);
    assert_eq!(summary.snapshot_id.len(), 64);
    assert!(summary.data_added > 0);
    assert!(!result.progress.is_empty() || summary.total_duration_seconds >= 0.0);

    std::fs::write(data.join("a.txt"), "first file, changed").expect("cannot write the file");
    // The limits, the retry-lock duration and the low priority must be options that restic accepts.
    let limited = make_step(
        "backup",
        step::Kind::Backup(BackupStep {
            limit_upload_kib: 100_000,
            limit_download_kib: 100_000,
            retry_lock: true,
            priority: ProcessPriority::Low as i32,
            ..backup("repo-1", &[&data_path])
        }),
    );
    let second = job(
        "job-maintenance",
        vec![access],
        vec![
            limited,
            snapshots_step("repo-1"),
            check_step("repo-1"),
            forget_step("repo-1", 1),
            prune_step("repo-1"),
            stats_step("repo-1"),
            unlock_step("repo-1"),
        ],
    );
    let result = run(&jobs, &second).await;
    let statuses: Vec<(String, i32)> = result
        .steps
        .iter()
        .map(|step| (step.step_id.clone(), step.status))
        .collect();
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{statuses:?}\n{}",
        result.log_text()
    );
    assert_eq!(result.steps.len(), 7);
    for step in &result.steps {
        assert_eq!(step.status, StepStatus::Success as i32, "{}", step.step_id);
    }
    let Some(Output::BackupSummary(summary)) = result.step("backup").output.clone() else {
        panic!("the backup must give a typed summary");
    };
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.files_unmodified, 1);
    let Some(Output::SnapshotList(list)) = result.step("snapshots").output.clone() else {
        panic!("snapshots must give a list");
    };
    assert_eq!(list.snapshots.len(), 2);
    let Some(Output::CheckSummary(check)) = result.step("check").output.clone() else {
        panic!("check must give a summary");
    };
    assert_eq!(check.num_errors, 0);
    let Some(Output::ForgetResult(forget)) = result.step("forget").output.clone() else {
        panic!("forget must give a result");
    };
    assert_eq!(forget.groups.len(), 1);
    assert_eq!(forget.groups[0].kept.len(), 1);
    assert_eq!(forget.groups[0].removed.len(), 1);
    let Some(Output::StatsSummary(stats)) = result.step("stats").output.clone() else {
        panic!("stats must give a summary");
    };
    assert_eq!(stats.snapshots_count, 1, "forget removed one snapshot");
    assert!(stats.total_size > 0);
    assert!(!result.log_text().contains("CANARY-real-steps"));

    // A wrong password gives config_error and never reaches the log (AC-AG-05).
    let wrong = job(
        "job-wrong",
        vec![self::access("repo-1", &repository, "another-password")],
        vec![snapshots_step("repo-1")],
    );
    let result = run(&jobs, &wrong).await;
    assert_eq!(
        result.step("snapshots").status,
        StepStatus::ConfigError as i32
    );
    assert_eq!(result.step("snapshots").exit_code, 12);
    assert_eq!(result.result().outcome, JobStatus::ConfigError as i32);
    assert!(!result.log_text().contains("another-password"));

    let _ = std::fs::remove_dir_all(&agent_dir);
    let _ = std::fs::remove_dir_all(&repository);
    let _ = std::fs::remove_dir_all(&data);
}
