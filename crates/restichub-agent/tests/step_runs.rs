//! The backup and maintenance steps against a scripted restic, without a control plane (FR-PR-02,
//! FR-AG-08, FR-AG-09, FR-AG-15, FR-PR-10, ADR-0021).
//!
//! The tests drive [`Jobs::handle`] directly and read the protocol messages of the stream channel.
//! The scripted `fake-restic` example prints the JSON lines and exits with the code that the test
//! writes next to the repository path.

#![allow(clippy::expect_used)]

mod common;

use std::path::PathBuf;

use common::*;
use restichub_proto::agent::v1::{
    BackupStep, JobStatus, LogStream, RejectReason, StepStatus, agent_message, step,
    step_result::Output,
};
use tokio::sync::mpsc;

type StepBuilder = fn() -> restichub_proto::agent::v1::Step;

const PASSWORD: &str = "CANARY-step-password";

/// A temporary repository path with its fake control files and an agent directory.
struct Fixture {
    dir: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = temp_config_dir(&format!("{name}-agent"));
        std::fs::create_dir_all(&dir).expect("cannot create the configuration directory");
        let path = temp_config_dir(&format!("{name}-repo"));
        std::fs::create_dir_all(&path).expect("cannot create the repository path");
        Self { dir, path }
    }

    fn jobs(&self) -> restichub_agent::jobs::Jobs {
        program_jobs(&self.dir)
    }

    fn repositories(&self) -> Vec<restichub_proto::agent::v1::RepositoryAccess> {
        vec![access("repo-1", &self.path, PASSWORD)]
    }

    fn job(
        &self,
        job_id: &str,
        steps: Vec<restichub_proto::agent::v1::Step>,
    ) -> restichub_proto::agent::v1::Job {
        job(job_id, self.repositories(), steps)
    }

    fn argv(&self) -> Vec<String> {
        recorded_arguments(&self.path)
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

const SUMMARY: &str = r#"{"message_type":"summary","files_new":3,"files_changed":1,"files_unmodified":10,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":4,"data_blobs":5,"tree_blobs":2,"data_added":2048,"data_added_packed":1024,"total_files_processed":14,"total_bytes_processed":9000,"total_duration":1.5,"snapshot_id":"abcdef0123456789"}"#;

/// AC-AG-05: every step kind maps the restic exit codes to the statuses of FR-AG-09.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_05_every_step_kind_maps_the_exit_codes() {
    let expected = [
        (0, StepStatus::Success),
        (1, StepStatus::Failed),
        (3, StepStatus::Warning),
        (10, StepStatus::ConfigError),
        (11, StepStatus::Retry),
        (12, StepStatus::ConfigError),
        (130, StepStatus::Cancelled),
        (99, StepStatus::Failed),
    ];
    let kinds: [(&str, StepBuilder); 7] = [
        ("backup", || backup_step("repo-1", &["/srv"])),
        ("check", || check_step("repo-1")),
        ("snapshots", || snapshots_step("repo-1")),
        ("forget", || forget_step("repo-1", 1)),
        ("prune", || prune_step("repo-1")),
        ("unlock", || unlock_step("repo-1")),
        ("stats", || stats_step("repo-1")),
    ];
    let fixture = Fixture::new("step-codes");
    let jobs = fixture.jobs();
    let mut index = 0;
    for (command, build) in kinds {
        for (code, status) in expected {
            script_code(&fixture.path, command, code);
            index += 1;
            let result = run(&jobs, &fixture.job(&format!("job-{index}"), vec![build()])).await;
            let step = &result.steps[0];
            assert_eq!(step.status, status as i32, "{command} exit {code}");
            assert_eq!(step.exit_code, code, "{command} exit {code}");
            let outcome = result.result().outcome;
            match status {
                StepStatus::Success => {
                    assert_eq!(
                        outcome,
                        JobStatus::Succeeded as i32,
                        "{command} exit {code}"
                    );
                }
                // A warning does not fail the job, but the job result names it.
                StepStatus::Warning => {
                    assert_eq!(
                        outcome,
                        JobStatus::SucceededWithWarnings as i32,
                        "{command} exit {code}"
                    );
                }
                StepStatus::Retry => {
                    assert_eq!(
                        outcome,
                        JobStatus::RepositoryLocked as i32,
                        "{command} exit {code}"
                    );
                    assert_eq!(
                        result
                            .result()
                            .error
                            .as_ref()
                            .map(|error| error.code.as_str()),
                        Some("repository_locked"),
                        "{command} exit {code}"
                    );
                }
                StepStatus::Cancelled => {
                    assert_eq!(
                        outcome,
                        JobStatus::Cancelled as i32,
                        "{command} exit {code}"
                    );
                }
                StepStatus::ConfigError if code == 12 => {
                    assert_eq!(
                        outcome,
                        JobStatus::ConfigError as i32,
                        "{command} exit {code}"
                    );
                }
                _ => assert_eq!(outcome, JobStatus::Failed as i32, "{command} exit {code}"),
            }
            if status != StepStatus::Success {
                assert!(step.error.is_some(), "{command} exit {code}: no error code");
            }
        }
    }
}

/// AC-AG-05: the job result carries the status of the step that decided it.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_05_the_job_result_names_a_warning_and_a_retry() {
    let fixture = Fixture::new("step-job-status");
    let jobs = fixture.jobs();
    script_code(&fixture.path, "backup", 3);
    let warned = run(
        &jobs,
        &fixture.job(
            "job-warn",
            vec![backup_step("repo-1", &["/srv"]), check_step("repo-1")],
        ),
    )
    .await;
    assert_eq!(warned.result().status, StepStatus::Warning as i32);
    assert_eq!(
        warned.result().outcome,
        JobStatus::SucceededWithWarnings as i32
    );
    assert_eq!(
        warned.result().steps_succeeded,
        2,
        "a warning does not stop the job"
    );
    assert_eq!(warned.steps.len(), 2);

    script_code(&fixture.path, "backup", 11);
    let locked = run(
        &jobs,
        &fixture.job(
            "job-lock",
            vec![backup_step("repo-1", &["/srv"]), check_step("repo-1")],
        ),
    )
    .await;
    assert_eq!(locked.result().status, StepStatus::Retry as i32);
    assert_eq!(locked.result().outcome, JobStatus::RepositoryLocked as i32);
    assert_eq!(locked.steps.len(), 1, "a locked repository stops the job");
}

/// FR-PR-10, ADR-0021, FR-AG-15: the compiler builds the arguments; `--json`, the limits and the
/// retry-lock duration reach restic; the positional paths follow `--`.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0021_the_backup_arguments_come_from_the_compiler() {
    let fixture = Fixture::new("step-backup-argv");
    let jobs = fixture.jobs();
    let step = make_step(
        "backup",
        step::Kind::Backup(BackupStep {
            excludes: vec!["*.tmp".to_owned()],
            tags: vec!["nightly".to_owned()],
            limit_upload_kib: 1024,
            limit_download_kib: 2048,
            retry_lock: true,
            ..backup("repo-1", &["--password-command=calc", "-x"])
        }),
    );
    let result = run(&jobs, &fixture.job("job-argv", vec![step])).await;
    assert_eq!(result.result().outcome, JobStatus::Succeeded as i32);
    assert_eq!(
        fixture.argv(),
        vec![
            "backup --json --exclude=*.tmp --tag=nightly --limit-upload=1024 \
             --limit-download=2048 --retry-lock=5m -- --password-command=calc -x"
        ]
    );
}

/// FR-PR-02: `unlock` removes stale locks only; `--remove-all` belongs to the `unlock_all` step.
#[tokio::test(flavor = "multi_thread")]
async fn fr_pr_02_only_unlock_all_removes_live_locks() {
    let fixture = Fixture::new("step-unlock");
    let jobs = fixture.jobs();
    run(
        &jobs,
        &fixture.job("job-unlock", vec![unlock_step("repo-1")]),
    )
    .await;
    run(
        &jobs,
        &fixture.job("job-all", vec![unlock_all_step("repo-1")]),
    )
    .await;
    assert_eq!(fixture.argv(), vec!["unlock", "unlock --remove-all"]);
}

/// FR-PR-02: the `ls`, `restore` and `hook` steps are refused until their epics.
#[tokio::test(flavor = "multi_thread")]
async fn fr_pr_02_the_later_step_kinds_are_rejected() {
    use restichub_proto::agent::v1::{HookStep, LsStep, RestoreStep};
    let fixture = Fixture::new("step-unsupported");
    let jobs = fixture.jobs();
    let kinds = [
        step::Kind::Ls(LsStep {
            repository_id: "repo-1".to_owned(),
            snapshot_id: "latest".to_owned(),
            ..LsStep::default()
        }),
        step::Kind::Restore(RestoreStep {
            repository_id: "repo-1".to_owned(),
            snapshot_id: "latest".to_owned(),
            target: "/restore".to_owned(),
            ..RestoreStep::default()
        }),
        step::Kind::Hook(HookStep {
            hook_id: "pg_dump".to_owned(),
            ..HookStep::default()
        }),
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let (tx, mut rx) = mpsc::channel(8);
        jobs.attach(&tx);
        let job = fixture.job(
            &format!("job-later-{index}"),
            vec![make_step("later", kind)],
        );
        jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
        let message = rx.recv().await.expect("the agent must answer");
        match message.payload {
            Some(agent_message::Payload::JobRejected(rejected)) => assert_eq!(
                RejectReason::try_from(rejected.reason).ok(),
                Some(RejectReason::UnsupportedStep)
            ),
            other => panic!("the job must be rejected: {other:?}"),
        }
    }
    assert!(fixture.argv().is_empty(), "no restic may start");
}

/// FR-AG-08: a backup sends progress and a typed summary, and the progress is throttled.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_a_backup_sends_throttled_progress_and_a_typed_summary() {
    let fixture = Fixture::new("step-progress");
    let jobs = fixture.jobs();
    let status = |done: u64| {
        format!(
            r#"{{"message_type":"status","percent_done":0.5,"total_files":14,"files_done":{done},"total_bytes":9000,"bytes_done":{done},"current_files":["/srv/a"]}}"#
        )
    };
    // A burst in one instant gives one progress message; a status 2.2 s later gives the next.
    let mut text = String::new();
    for done in 1..=40 {
        text.push_str(&status(done));
        text.push('\n');
    }
    text.push_str("#sleep 2200\n");
    text.push_str(&status(99));
    text.push('\n');
    // The final status needs a poll turn before restic exits; the summary replaces it after that.
    text.push_str("#sleep 300\n");
    text.push_str(SUMMARY);
    text.push('\n');
    script_stdout(&fixture.path, "backup", &text);

    let result = run(
        &jobs,
        &fixture.job("job-progress", vec![backup_step("repo-1", &["/srv"])]),
    )
    .await;
    assert_eq!(result.progress.len(), 2, "{:?}", result.progress);
    assert_eq!(result.progress[0].job_id, "job-progress");
    assert_eq!(result.progress[0].step_id, "backup");
    assert_eq!(result.progress[0].total_files, 14);
    // The second message is the newest status at the 2 s mark: the held end of the burst or the late one.
    assert!(result.progress[1].files_done >= 40);
    assert!(result.progress[1].files_done > result.progress[0].files_done);
    assert_eq!(result.progress[0].current_files, vec!["/srv/a".to_owned()]);

    let Some(Output::BackupSummary(summary)) = result.step("backup").output.clone() else {
        panic!("the backup step must carry the summary");
    };
    assert_eq!(summary.files_new, 3);
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.files_unmodified, 10);
    assert_eq!(summary.data_added, 2048);
    assert_eq!(summary.snapshot_id, "abcdef0123456789");
    assert!((summary.total_duration_seconds - 1.5).abs() < f64::EPSILON);
}

/// FR-AG-08: check, snapshots, forget and stats give typed results.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_the_maintenance_steps_give_typed_results() {
    let fixture = Fixture::new("step-typed");
    let jobs = fixture.jobs();
    script_stdout(
        &fixture.path,
        "check",
        r#"{"message_type":"summary","num_errors":2,"broken_packs":["a"],"suggest_repair_index":true,"suggest_prune":false}"#,
    );
    script_stdout(
        &fixture.path,
        "snapshots",
        r#"[{"time":"2026-10-14T10:00:00Z","tree":"t1","paths":["/srv"],"hostname":"h","username":"u","tags":["a"],"id":"i1","short_id":"i1"}]"#,
    );
    script_stdout(
        &fixture.path,
        "forget",
        r#"[{"tags":null,"host":"h","paths":["/srv"],"keep":[{"time":"2026-10-14T10:00:00Z","id":"k1"}],"remove":[{"time":"2026-10-13T10:00:00Z","id":"r1"}],"reasons":[{"snapshot":{"id":"k1"},"matches":["last snapshot"]}]}]"#,
    );
    script_stdout(
        &fixture.path,
        "stats",
        r#"{"total_size":100,"total_uncompressed_size":200,"total_file_count":7,"compression_ratio":2.0,"snapshots_count":3}"#,
    );
    let steps = vec![
        check_step("repo-1"),
        snapshots_step("repo-1"),
        forget_step("repo-1", 1),
        stats_step("repo-1"),
    ];
    let result = run(&jobs, &fixture.job("job-typed", steps)).await;
    assert_eq!(result.result().outcome, JobStatus::Succeeded as i32);
    let Some(Output::CheckSummary(check)) = result.step("check").output.clone() else {
        panic!("check output");
    };
    assert_eq!(check.num_errors, 2);
    assert!(check.repair_suggested);
    let Some(Output::SnapshotList(list)) = result.step("snapshots").output.clone() else {
        panic!("snapshots output");
    };
    assert_eq!(list.snapshots[0].id, "i1");
    assert_eq!(list.snapshots[0].tags, vec!["a".to_owned()]);
    let Some(Output::ForgetResult(forget)) = result.step("forget").output.clone() else {
        panic!("forget output");
    };
    assert_eq!(forget.groups[0].kept[0].id, "k1");
    assert_eq!(forget.groups[0].kept[0].reason, "last snapshot");
    assert_eq!(forget.groups[0].removed[0].id, "r1");
    let Some(Output::StatsSummary(stats)) = result.step("stats").output.clone() else {
        panic!("stats output");
    };
    assert_eq!(stats.total_size, 100);
    assert_eq!(stats.snapshots_count, 3);
}

/// FR-AG-15 (Linux): a step with the low priority starts restic with nice 10 and the lowest
/// best-effort I/O class.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_15_a_low_priority_step_lowers_the_process_priority() {
    let fixture = Fixture::new("step-priority");
    let jobs = fixture.jobs();
    let step = make_step(
        "backup",
        step::Kind::Backup(BackupStep {
            priority: restichub_proto::agent::v1::ProcessPriority::Low as i32,
            ..backup("repo-1", &["/srv"])
        }),
    );
    run(&jobs, &fixture.job("job-low", vec![step])).await;
    let state = recorded_priority(&fixture.path);
    assert_eq!(state.trim(), "nice=10 io_class=2 io_level=7");
}

/// FR-AG-08: restic text is scrubbed, and a log over the limit ends with a marker.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_the_log_is_scrubbed_and_bounded() {
    let fixture = Fixture::new("step-log");
    let jobs = fixture.jobs();
    script_stderr(
        &fixture.path,
        "backup",
        "Fatal: wrong password {PASSWORD} for the repository\nsecond line\n",
    );
    let mut text = String::from("{PASSWORD} on stdout\n");
    let filler = "x".repeat(499);
    for _ in 0..3_000 {
        text.push_str(&filler);
        text.push('\n');
    }
    script_stdout(&fixture.path, "prune", &text);
    let steps = vec![backup_step("repo-1", &["/srv"]), prune_step("repo-1")];
    let result = run(&jobs, &fixture.job("job-log", steps)).await;
    let log = result.log_text();
    assert!(!log.contains(PASSWORD), "the password reached the log");
    assert!(log.contains("Fatal: wrong password *** for the repository"));
    assert!(log.contains("*** on stdout"));
    assert!(log.contains("log truncated"));
    assert!(log.len() <= 1024 * 1024 + 200, "{}", log.len());
    let marker = result.logs.last().expect("the last chunk");
    assert_eq!(marker.stream, LogStream::Agent as i32);
    let backup_logs: Vec<_> = result
        .logs
        .iter()
        .filter(|chunk| chunk.step_id == "backup")
        .collect();
    assert_eq!(backup_logs[0].stream, LogStream::Stderr as i32);
    assert_eq!(backup_logs[0].job_id, "job-log");
}
