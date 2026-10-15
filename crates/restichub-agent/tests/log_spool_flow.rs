//! The log of a job reaches the control plane without delaying a cancel, and the end of the log
//! comes before the job result (FR-LG-07, FR-AG-10, AC-LG-05).

#![allow(clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use common::spool::*;
use common::*;
use restichub_agent::log_spool::{LogSpool, SPOOL_DIR};
use restichub_proto::agent::v1::{AgentMessage, agent_message};
use tokio::sync::mpsc;

/// FR-LG-07: the last lines of a job reach the control plane before its job result, also when
/// two jobs run at once and the stream buffer is small. No reconnect is needed.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_the_end_of_the_log_arrives_before_the_job_result_without_a_reconnect() {
    let dir = TempDir::new("spool-end-agent");
    let repo = TempDir::new("spool-end-repo");
    script_stdout(&repo, "backup", &lines_script("line", 300));
    let jobs = program_jobs(&dir);

    let (tx, mut rx) = mpsc::channel::<AgentMessage>(8);
    jobs.attach(&tx);
    jobs.greeted(true);
    let reader = tokio::spawn(async move {
        let mut logs: HashMap<String, StoredLog> = HashMap::new();
        let mut at_result: HashMap<String, usize> = HashMap::new();
        while let Some(message) = rx.recv().await {
            match message.payload {
                Some(agent_message::Payload::LogChunk(chunk)) => {
                    logs.entry(chunk.job_id.clone()).or_default().take(&chunk);
                }
                Some(agent_message::Payload::JobResult(result)) => {
                    // The log of a step opens with the command line, which is no line of the script.
                    let lines = logs.get(&result.job_id).map_or(0, |log| {
                        log.lines()
                            .iter()
                            .filter(|line| !line.starts_with("$ "))
                            .count()
                    });
                    at_result.insert(result.job_id, lines);
                    if at_result.len() == 2 {
                        break;
                    }
                }
                _ => {}
            }
            // A control plane that is slower than the agent.
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        at_result
    });
    start_job(&jobs, &repo, "job-end-a").await;
    start_job(&jobs, &repo, "job-end-b").await;
    let at_result = tokio::time::timeout(Duration::from_secs(60), reader)
        .await
        .expect("both job results arrive")
        .expect("the reader stops cleanly");
    assert_eq!(at_result.get("job-end-a"), Some(&300));
    assert_eq!(at_result.get("job-end-b"), Some(&300));
}

/// Starts a long job against a control plane that reads one message in `delay`, then cancels it.
/// The cancel takes effect in seconds, and the agent keeps room in the buffer for its other
/// messages.
async fn cancel_with_reader_delay(name: &str, delay: Duration) {
    let dir = TempDir::new(&format!("spool-{name}-agent"));
    let repo = TempDir::new(&format!("spool-{name}-repo"));
    script_stdout(
        &repo,
        "backup",
        &format!("{}#sleep 30000\n", lines_script("busy", 400)),
    );
    let jobs = program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel::<AgentMessage>(8);
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if matches!(message.payload, Some(agent_message::Payload::LogChunk(_))) {
                tokio::time::sleep(delay).await;
            }
        }
    });
    jobs.attach(&tx);
    jobs.greeted(true);
    start_job(&jobs, &repo, name).await;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        tx.capacity() >= 2,
        "the log leaves room for the other messages: {}",
        tx.capacity()
    );
    let started = Instant::now();
    jobs.cancel(name);
    wait_for_result(&dir).await;
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
}

/// FR-LG-07, FR-AG-10: a control plane that reads nothing does not delay a cancel.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_10_a_stuck_stream_does_not_delay_a_cancel() {
    cancel_with_reader_delay("stuck", Duration::from_secs(60)).await;
}

/// FR-AG-10: a slow but live control plane does not delay a cancel either.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_10_a_slow_stream_does_not_delay_a_cancel() {
    cancel_with_reader_delay("slow", Duration::from_millis(200)).await;
}

/// FR-LG-07: the greeting returns at once, however large the spool is and however slow the control
/// plane reads, so the stream task goes on with the pings and the cancels.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_10_the_greeting_does_not_wait_for_the_spool() {
    let dir = TempDir::new("spool-greeting-agent");
    let old = LogSpool::open(&dir, 64 * 1024 * 1024);
    for sequence in 1..=2_000 {
        old.append(
            "old-job",
            &restichub_proto::agent::v1::LogChunk {
                job_id: "old-job".to_owned(),
                step_id: "backup".to_owned(),
                stream: restichub_proto::agent::v1::LogStream::Stdout as i32,
                sequence,
                data: b"line\n".to_vec(),
            },
        )
        .expect("append");
    }
    old.finish("old-job");
    drop(old);

    let jobs = program_jobs(&dir);
    let (tx, _rx) = mpsc::channel::<AgentMessage>(8);
    jobs.attach(&tx);
    let started = Instant::now();
    jobs.send_stored_results().await;
    jobs.greeted(true);
    assert!(started.elapsed() < Duration::from_millis(500));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(tx.capacity() >= 2, "{}", tx.capacity());
    assert!(dir.join(SPOOL_DIR).join("old-job.spool").exists());
}

/// FR-LG-07: a full spool drops lines and says how many. The marker does not claim a cause that
/// the agent does not know.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_full_spool_leaves_a_marker_with_the_count() {
    let dir = TempDir::new("spool-cap-agent");
    let repo = TempDir::new("spool-cap-repo");
    let total = 3_000;
    script_stdout(
        &repo,
        "backup",
        &lines_script("line-with-some-padding-to-fill", total),
    );
    let jobs = program_jobs(&dir).with_log_spool(LogSpool::open(&dir, 8 * 1024));
    let (tx, mut rx) = mpsc::channel::<AgentMessage>(64);
    jobs.attach(&tx);
    jobs.greeted(true);
    start_job(&jobs, &repo, "job-cap").await;
    rx.recv().await.expect("the agent acknowledges the job");
    drop(rx);
    wait_for_result(&dir).await;

    let (_tx, mut rx) = connect(&jobs, 4096).await;
    jobs.greeted(true);
    let mut stored = StoredLog::default();
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    drain(&mut rx, &mut stored);
    let lines = stored.lines();
    let lost: usize = lines
        .iter()
        .filter(|line| line.ends_with("because the agent log spool was full or not writable"))
        .map(|line| {
            line.split(' ')
                .next()
                .and_then(|count| count.parse::<usize>().ok())
                .expect("a marker starts with the count")
        })
        .sum();
    let kept = lines
        .iter()
        .filter(|line| line.starts_with("line-with-some-padding-to-fill"))
        .count();
    assert!(kept > 0 && lost > 0);
    // The agent also adds its own lines to the log; the count covers only the lines of restic.
    assert_eq!(kept + lost, total);
    assert!(
        std::fs::metadata(dir.join(SPOOL_DIR).join("job-cap.spool"))
            .expect("the spool file exists")
            .len()
            < 9 * 1024
    );
}
