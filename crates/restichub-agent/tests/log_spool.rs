//! The agent keeps unacknowledged log chunks and sends them again after a reconnect or a restart,
//! in order (FR-LG-07, AC-LG-05, AC-PR-07).

#![allow(clippy::expect_used)]

mod common;

use std::path::Path;

use common::spool::*;
use common::*;
use restichub_agent::log_spool::{LogSpool, SPOOL_DIR};
use restichub_proto::agent::v1::{LogChunk, LogStream, agent_message};

fn numbered(prefix: &str, range: std::ops::RangeInclusive<usize>) -> Vec<String> {
    range.map(|n| format!("{prefix}-{n:04}")).collect()
}

/// The lines of the test, without the command line that opens the log of a step.
fn owned(lines: Vec<&str>) -> Vec<String> {
    lines
        .into_iter()
        .filter(|line| !line.starts_with("$ "))
        .map(str::to_owned)
        .collect()
}

/// Starts a job while a stream is open, then drops the stream: the control plane goes away.
async fn start_job_then_lose_the_stream(jobs: &restichub_agent::jobs::Jobs, repo: &Path, id: &str) {
    let (tx, mut rx) = connect(jobs, 64).await;
    jobs.greeted(true);
    start_job(jobs, repo, id).await;
    let ack = rx.recv().await.expect("the agent acknowledges the job");
    assert!(matches!(
        ack.payload,
        Some(agent_message::Payload::JobAck(_))
    ));
    drop(rx);
    drop(tx);
}

/// FR-LG-07, AC-PR-07: the stream drops in the middle of a job. After the reconnect the control
/// plane has every line once and in order, and the acknowledged data leaves the disk.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_stream_drop_mid_job_loses_no_line_and_repeats_none() {
    let dir = TempDir::new("spool-drop-agent");
    let repo = TempDir::new("spool-drop-repo");
    let script = format!(
        "{}#sleep 3000\n{}",
        lines_script("first", 60),
        lines_script("second", 60)
    );
    script_stdout(&repo, "backup", &script);
    let jobs = program_jobs(&dir);

    let (tx, mut rx) = connect(&jobs, 4096).await;
    jobs.greeted(true);
    start_job(&jobs, &repo, "job-drop").await;
    let mut stored = StoredLog::default();
    collect_lines(&mut rx, &mut stored, 60).await;
    jobs.log_ack(&stored.ack("job-drop"));
    // The stream drops: the job runs on, and its chunks wait in the spool.
    drop(rx);
    drop(tx);
    wait_for_result(&dir).await;

    let (_tx, mut rx) = connect(&jobs, 4096).await;
    jobs.greeted(true);
    collect_lines(&mut rx, &mut stored, 120).await;
    let expected: Vec<String> = numbered("first", 1..=60)
        .into_iter()
        .chain(numbered("second", 1..=60))
        .collect();
    assert_eq!(owned(stored.lines()), expected);

    jobs.log_ack(&stored.ack("job-drop"));
    wait_for_spool_files(&dir, 0).await;
}

/// FR-LG-07: the agent restarts and the control plane never saw the log. The new process sends the
/// spool after the greeting.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_restart_sends_the_unacknowledged_spool() {
    let dir = TempDir::new("spool-restart-agent");
    let repo = TempDir::new("spool-restart-repo");
    script_stdout(&repo, "backup", &lines_script("line", 40));
    let jobs = program_jobs(&dir);
    start_job_then_lose_the_stream(&jobs, &repo, "job-restart").await;
    wait_for_result(&dir).await;
    assert_eq!(spool_files(&dir), 1);
    drop(jobs);

    let restarted = program_jobs(&dir);
    let (_tx, mut rx) = connect(&restarted, 4096).await;
    restarted.greeted(true);
    let mut stored = StoredLog::default();
    collect_lines(&mut rx, &mut stored, 41).await;
    let received = owned(stored.lines());
    assert_eq!(received[0], "line-0001");
    assert_eq!(received[39], "line-0040");
    // A second connection sends the same chunks again until the control plane acknowledges them.
    let (_tx2, mut rx2) = connect(&restarted, 4096).await;
    restarted.greeted(true);
    let mut again = StoredLog::default();
    collect_lines(&mut rx2, &mut again, 41).await;

    restarted.log_ack(&stored.ack("job-restart"));
    wait_for_spool_files(&dir, 0).await;
}

/// FR-LG-07: from the attach of a stream until the greeting, the lines of a running job join the
/// spool behind the older ones. The control plane gets them in the original order, and nothing
/// goes out before the greeting.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_lines_written_before_the_greeting_keep_their_order() {
    let dir = TempDir::new("spool-window-agent");
    let repo = TempDir::new("spool-window-repo");
    let script = format!(
        "{}#sleep 2500\n{}",
        lines_script("early", 30),
        lines_script("late", 30)
    );
    script_stdout(&repo, "backup", &script);
    let jobs = program_jobs(&dir);

    let (tx, mut rx) = connect(&jobs, 4096).await;
    jobs.greeted(true);
    start_job(&jobs, &repo, "job-window").await;
    let mut first = StoredLog::default();
    collect_lines(&mut rx, &mut first, 30).await;
    // The control plane never acknowledges, and the stream drops.
    drop(rx);
    drop(tx);

    // The new stream is attached, but the greeting did not come yet.
    let (_tx2, mut rx2) = connect(&jobs, 4096).await;
    wait_for_result(&dir).await;
    let mut early = StoredLog::default();
    drain(&mut rx2, &mut early);
    assert!(
        early.lines().is_empty(),
        "no chunk goes out before the greeting: {:?}",
        early.lines()
    );

    jobs.greeted(true);
    let mut stored = StoredLog::default();
    collect_lines(&mut rx2, &mut stored, 60).await;
    let expected: Vec<String> = numbered("early", 1..=30)
        .into_iter()
        .chain(numbered("late", 1..=30))
        .collect();
    assert_eq!(owned(stored.lines()), expected);
}

/// FR-LG-07: an acknowledgement removes only the chunks that went out, so a chunk that waits in
/// the spool is never deleted by an acknowledgement of a later one.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_an_early_acknowledgement_does_not_delete_a_chunk_that_was_not_sent() {
    let dir = TempDir::new("spool-early-ack-agent");
    let repo = TempDir::new("spool-early-ack-repo");
    script_stdout(&repo, "backup", &lines_script("line", 20));
    let jobs = program_jobs(&dir);
    start_job_then_lose_the_stream(&jobs, &repo, "job-early").await;
    wait_for_result(&dir).await;

    // A stream is attached, and the greeting did not come: the chunks have not gone out. An
    // acknowledgement for them covers nothing.
    let (_tx, mut rx) = connect(&jobs, 4096).await;
    let mut covered = StoredLog::default();
    covered.take(&LogChunk {
        job_id: "job-early".to_owned(),
        step_id: "repo-1".to_owned(),
        stream: LogStream::Stdout as i32,
        sequence: 1_000,
        data: Vec::new(),
    });
    jobs.log_ack(&covered.ack("job-early"));
    assert_eq!(spool_files(&dir), 1);

    jobs.greeted(true);
    let mut stored = StoredLog::default();
    collect_lines(&mut rx, &mut stored, 20).await;
}

/// FR-LG-07: a spool whose job left the journal is sent, not deleted without a trace.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_spool_of_an_expired_job_is_sent() {
    let dir = TempDir::new("spool-old-agent");
    let old = LogSpool::open(&dir, 1024 * 1024);
    old.append(
        "old-job",
        &LogChunk {
            job_id: "old-job".to_owned(),
            step_id: "backup".to_owned(),
            stream: LogStream::Stdout as i32,
            sequence: 1,
            data: b"from the weekend\n".to_vec(),
        },
    )
    .expect("append");
    old.finish("old-job");
    drop(old);

    let jobs = program_jobs(&dir);
    let (_tx, mut rx) = connect(&jobs, 64).await;
    jobs.greeted(true);
    let mut stored = StoredLog::default();
    collect_lines(&mut rx, &mut stored, 1).await;
    assert_eq!(stored.lines(), ["from the weekend"]);
    assert!(dir.join(SPOOL_DIR).join("old-job.spool").exists());
    jobs.log_ack(&stored.ack("old-job"));
    wait_for_spool_files(&dir, 0).await;
}

/// ADR-0042: a control plane without the capability gets the spool once and the agent keeps no
/// copy, as it did before the spool.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_control_plane_without_the_capability_gets_the_spool_once() {
    let dir = TempDir::new("spool-old-cp-agent");
    let repo = TempDir::new("spool-old-cp-repo");
    script_stdout(&repo, "backup", &lines_script("line", 40));
    let jobs = program_jobs(&dir);
    start_job_then_lose_the_stream(&jobs, &repo, "job-old-cp").await;
    wait_for_result(&dir).await;
    assert_eq!(spool_files(&dir), 1);

    let (_tx, mut rx) = connect(&jobs, 4096).await;
    jobs.greeted(false);
    let mut stored = StoredLog::default();
    collect_lines(&mut rx, &mut stored, 40).await;
    wait_for_spool_files(&dir, 0).await;
}
