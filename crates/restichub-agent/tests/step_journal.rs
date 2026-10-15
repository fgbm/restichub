//! The result of a job that finishes while the control plane is away, and the canary checks of the
//! agent (AC-PR-07, AC-AG-03, AC-AG-04, FR-AG-04, FR-AG-06).

#![allow(clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use prost::Message as _;
use restichub_canary_scan::{Canary, Scanner};
use restichub_proto::agent::v1::{JobStatus, StepStatus, agent_message, step_result::Output};
use tokio::sync::mpsc;

const SUMMARY: &str = r#"{"message_type":"summary","files_new":4,"files_changed":0,"files_unmodified":0,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":0,"data_blobs":4,"tree_blobs":1,"data_added":4096,"data_added_packed":2048,"total_files_processed":4,"total_bytes_processed":8000,"total_duration":0.5,"snapshot_id":"feedface"}"#;

fn directory(name: &str) -> PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

/// AC-PR-07: the control plane is away while the job runs. The job completes, and the typed result
/// with the log tail reaches the control plane as soon as the stream is back, also after a restart.
#[tokio::test(flavor = "multi_thread")]
async fn ac_pr_07_a_result_that_finished_offline_arrives_after_the_reconnect() {
    let dir = directory("journal-offline-agent");
    let path = directory("journal-offline-repo");
    script_stdout(&path, "backup", &format!("{SUMMARY}\n"));
    script_stderr(
        &path,
        "backup",
        "warning: one file changed while it was read\n",
    );
    script_sleep(&path, 1_000);
    let jobs = program_jobs(&dir);

    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    let job = job(
        "job-offline",
        vec![access("repo-1", &path, "topsecret")],
        vec![backup_step("repo-1", &["/srv"])],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let ack = rx.recv().await.expect("the agent acknowledges the job");
    assert!(matches!(
        ack.payload,
        Some(agent_message::Payload::JobAck(_))
    ));
    // The control plane goes away: every later send fails.
    drop(rx);
    drop(tx);

    // The job completes without the control plane. The stream comes back and the agent sends the
    // stored result at once.
    let mut collected = None;
    for _ in 0..200 {
        let (tx, mut rx) = mpsc::channel(64);
        jobs.attach(&tx);
        jobs.send_stored_results().await;
        let mut messages = Vec::new();
        while let Ok(message) = rx.try_recv() {
            messages.push(message);
        }
        // The job that runs at this moment sends its result to the new stream, but its step
        // results went to the stream that is gone. The stored copy is a batch with a step result.
        let holds = |is: fn(&agent_message::Payload) -> bool| {
            messages
                .iter()
                .any(|message| message.payload.as_ref().is_some_and(is))
        };
        if holds(|payload| matches!(payload, agent_message::Payload::StepResult(_)))
            && holds(|payload| matches!(payload, agent_message::Payload::JobResult(_)))
        {
            collected = Some(messages);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut messages = collected.expect("the stored result must arrive");
    // The live job result can arrive before the stored copy; the replay stops at the first job
    // result, so the job results go last.
    messages.sort_by_key(|message| {
        matches!(message.payload, Some(agent_message::Payload::JobResult(_)))
    });
    let (tx, mut rx) = mpsc::channel(64);
    for message in messages {
        tx.send(message).await.expect("replay");
    }
    let replay = collect(&mut rx).await;
    assert_eq!(replay.result().outcome, JobStatus::Succeeded as i32);
    let step = replay.step("backup");
    assert_eq!(step.status, StepStatus::Success as i32);
    let Some(Output::BackupSummary(summary)) = step.output.clone() else {
        panic!("the stored step result must carry the backup summary");
    };
    assert_eq!(summary.files_new, 4);
    assert_eq!(summary.snapshot_id, "feedface");
    assert!(
        replay
            .log_text()
            .contains("warning: one file changed while it was read")
    );

    // After a restart of the agent the journal gives the same answer (FR-AG-12). The write of the
    // result runs off the async workers, so the restart waits until the file holds it.
    let journal_file = dir.join(restichub_agent::journal::JOURNAL_FILE);
    for _ in 0..200 {
        let text = std::fs::read_to_string(&journal_file).unwrap_or_default();
        if text.contains("\"result\":\"") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let restarted = program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(64);
    restarted.attach(&tx);
    restarted.send_stored_results().await;
    let again = collect(&mut rx).await;
    assert_eq!(again.result().job_id, "job-offline");
    assert!(matches!(
        again.step("backup").output,
        Some(Output::BackupSummary(_))
    ));
    assert_eq!(
        recorded_arguments(&path).lines().count(),
        1,
        "restic ran once"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// AC-PR-07: the journal keeps the small summaries and drops the large lists.
#[tokio::test(flavor = "multi_thread")]
async fn ac_pr_07_the_stored_copy_drops_the_snapshot_list() {
    let dir = directory("journal-compact-agent");
    let path = directory("journal-compact-repo");
    script_stdout(
        &path,
        "snapshots",
        r#"[{"time":"2026-10-14T10:00:00Z","tree":"t","paths":["/srv"],"hostname":"h","username":"u","id":"i1","short_id":"i1"}]"#,
    );
    script_stdout(
        &path,
        "check",
        r#"{"message_type":"summary","num_errors":0}"#,
    );
    let jobs = program_jobs(&dir);
    let job = job(
        "job-compact",
        vec![access("repo-1", &path, "topsecret")],
        vec![snapshots_step("repo-1"), check_step("repo-1")],
    );
    let live = run(&jobs, &job).await;
    assert!(matches!(
        live.step("snapshots").output,
        Some(Output::SnapshotList(_))
    ));

    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    jobs.send_stored_results().await;
    let stored = collect(&mut rx).await;
    assert!(stored.step("snapshots").output.is_none());
    assert!(matches!(
        stored.step("check").output,
        Some(Output::CheckSummary(_))
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// Counts the message kinds that one `send_stored_results` call sends: log chunks, step results
/// and job results.
async fn resent_kinds(jobs: &restichub_agent::jobs::Jobs) -> (usize, usize, usize) {
    use restichub_proto::agent::v1::agent_message::Payload;
    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    jobs.send_stored_results().await;
    let mut kinds = (0, 0, 0);
    while let Ok(message) = rx.try_recv() {
        match message.payload {
            Some(Payload::LogChunk(_)) => kinds.0 += 1,
            Some(Payload::StepResult(_)) => kinds.1 += 1,
            Some(Payload::JobResult(_)) => kinds.2 += 1,
            _ => {}
        }
    }
    kinds
}

/// ADR-0020: the control plane has no result acknowledgement, so a later connection sends the job
/// result alone. The log tail and the step results go out once for each process start.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0020_the_tail_and_the_steps_are_re_sent_once_for_each_process_start() {
    let dir = directory("journal-resend-agent");
    let path = directory("journal-resend-repo");
    script_stderr(&path, "backup", "warning: one file changed\n");
    let jobs = program_jobs(&dir);
    let job = job(
        "job-resend",
        vec![access("repo-1", &path, "topsecret")],
        vec![backup_step("repo-1", &["/srv"])],
    );
    let _ = run(&jobs, &job).await;

    let (chunks, steps, results) = resent_kinds(&jobs).await;
    assert!(chunks > 0 && steps == 1 && results == 1);
    assert_eq!(
        resent_kinds(&jobs).await,
        (0, 0, 1),
        "the second connection"
    );
    assert_eq!(resent_kinds(&jobs).await, (0, 0, 1), "the third connection");

    let restarted = program_jobs(&dir);
    let (chunks, steps, results) = resent_kinds(&restarted).await;
    assert!(chunks > 0 && steps == 1 && results == 1, "after a restart");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

fn scan_file(scanner: &Scanner, path: &Path) -> usize {
    scanner.scan_file(path).map_or(0, |found| found.len())
}

/// AC-AG-03: after ten jobs on three repositories, the canary is nowhere on the agent: not in the
/// state directory, the journal, the cache, the logs that the agent sent or the files of restic.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_03_ten_jobs_leave_no_canary_on_the_agent() {
    let canary = format!("CANARY-{}", uuid::Uuid::now_v7());
    let scanner = Scanner::new(&[Canary::parse(&canary)]);
    let dir = directory("canary-agent");
    let repositories: Vec<PathBuf> = (0..3)
        .map(|index| directory(&format!("canary-repo-{index}")))
        .collect();
    for path in &repositories {
        // The fake echoes the password to both pipes, as a hostile backend could.
        script_stdout(path, "prune", "prune used {PASSWORD}\n");
        script_stderr(path, "backup", "Fatal: {PASSWORD} is {PASSWORD}\n");
        script_stdout(path, "backup", &format!("{SUMMARY}\n"));
    }
    let jobs = program_jobs(&dir);
    let mut sent = Vec::new();
    for index in 0..10 {
        let path = &repositories[index % 3];
        let steps = vec![backup_step("repo-1", &["/srv"]), prune_step("repo-1")];
        let job = job(
            &format!("job-{index}"),
            vec![access("repo-1", path, &canary)],
            steps,
        );
        let collected = run(&jobs, &job).await;
        assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
        for message in &collected.messages {
            sent.extend_from_slice(&message.encode_to_vec());
        }
    }
    assert!(
        scanner.scan("messages", &sent).is_empty(),
        "the canary reached a message to the control plane"
    );
    let findings = scanner
        .scan_directory(&dir)
        .expect("cannot scan the agent directory");
    assert!(
        findings.is_empty(),
        "{} findings in the agent state",
        findings.len()
    );
    for path in &repositories {
        for suffix in ["argv", "state"] {
            let file = PathBuf::from(format!("{}.restichub-{suffix}", path.display()));
            assert_eq!(scan_file(&scanner, &file), 0, "{}", file.display());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    for path in repositories {
        let _ = std::fs::remove_dir_all(&path);
    }
}

/// AC-AG-04 (Linux): while a backup runs, no process argument holds the canary.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_04_no_process_argument_holds_the_canary_during_a_backup() {
    let canary = format!("CANARY-{}", uuid::Uuid::now_v7());
    let scanner = Scanner::new(&[Canary::parse(&canary)]);
    let dir = directory("cmdline-agent");
    let path = directory("cmdline-repo");
    script_sleep(&path, 3_000);
    let jobs = program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    let job = job(
        "job-cmdline",
        vec![access("repo-1", &path, &canary)],
        vec![backup_step("repo-1", &["/srv"])],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;

    assert!(
        wait_for_run(&path, "backup").await,
        "the backup must be running"
    );
    let mut checked = 0;
    for file in restichub_canary_scan::cmdline_paths(Path::new("/proc")).expect("cannot read /proc")
    {
        // Processes end while the scan runs.
        if let Ok(findings) = scanner.scan_file(&file) {
            assert!(findings.is_empty(), "{} holds the canary", file.display());
            checked += 1;
        }
    }
    assert!(checked > 0, "the scan read no process");
    let finished = collect(&mut rx).await;
    assert_eq!(finished.result().outcome, JobStatus::Succeeded as i32);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}
