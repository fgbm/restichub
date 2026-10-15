//! The agent keeps unacknowledged log chunks and sends them again after a reconnect or a restart
//! (FR-LG-07, AC-LG-05, AC-PR-07).

#![allow(clippy::expect_used)]

mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::*;
use restichub_agent::jobs::Jobs;
use restichub_agent::log_spool::{LogSpool, SPOOL_DIR};
use restichub_proto::agent::v1::{AgentMessage, LogAck, LogChunk, LogStreamAck, agent_message};
use tokio::sync::mpsc;

fn directory(name: &str) -> PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

/// The log of a job as the control plane stores it: a chunk once, in the order of arrival.
#[derive(Default)]
struct StoredLog {
    seen: HashSet<(i32, u64)>,
    text: String,
    chunks: Vec<LogChunk>,
}

impl StoredLog {
    fn take(&mut self, chunk: &LogChunk) {
        if self.seen.insert((chunk.stream, chunk.sequence)) {
            self.text.push_str(&String::from_utf8_lossy(&chunk.data));
            self.chunks.push(chunk.clone());
        }
    }

    fn lines(&self) -> Vec<&str> {
        self.text.lines().collect()
    }

    /// The acknowledgement of everything stored so far.
    fn ack(&self, job_id: &str) -> LogAck {
        let mut newest: Vec<(i32, u64)> = Vec::new();
        for chunk in &self.chunks {
            match newest
                .iter_mut()
                .find(|(stream, _)| *stream == chunk.stream)
            {
                Some(entry) => entry.1 = entry.1.max(chunk.sequence),
                None => newest.push((chunk.stream, chunk.sequence)),
            }
        }
        LogAck {
            job_id: job_id.to_owned(),
            streams: newest
                .into_iter()
                .map(|(stream, sequence)| LogStreamAck { stream, sequence })
                .collect(),
        }
    }
}

fn drain(rx: &mut mpsc::Receiver<AgentMessage>, stored: &mut StoredLog) -> bool {
    let mut result = false;
    while let Ok(message) = rx.try_recv() {
        match message.payload {
            Some(agent_message::Payload::LogChunk(chunk)) => stored.take(&chunk),
            Some(agent_message::Payload::JobResult(_)) => result = true,
            _ => {}
        }
    }
    result
}

async fn wait_for_result(dir: &Path) {
    let journal = dir.join(restichub_agent::journal::JOURNAL_FILE);
    for _ in 0..400 {
        if std::fs::read_to_string(&journal)
            .unwrap_or_default()
            .contains("\"result\":\"")
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the job did not finish");
}

fn spool_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join(SPOOL_DIR)).map_or(0, |entries| entries.flatten().count())
}

fn lines_script(prefix: &str, count: usize) -> String {
    (1..=count).map(|n| format!("{prefix}-{n:04}\n")).collect()
}

/// Starts a job while a stream is open, then drops the stream: the control plane goes away.
async fn start_job_then_lose_the_stream(jobs: &Jobs, repo: &Path, job_id: &str) {
    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    jobs.set_log_acks(true);
    start_job(jobs, repo, job_id).await;
    let ack = rx.recv().await.expect("the agent acknowledges the job");
    assert!(matches!(
        ack.payload,
        Some(agent_message::Payload::JobAck(_))
    ));
    drop(rx);
    drop(tx);
}

async fn start_job(jobs: &Jobs, repo: &Path, job_id: &str) {
    let job = job(
        job_id,
        vec![access("repo-1", repo, "topsecret")],
        vec![backup_step("repo-1", &["/srv"])],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
}

/// Connects like the stream task: the stored results first, then the greeting.
async fn reconnect(jobs: &Jobs) -> (mpsc::Sender<AgentMessage>, mpsc::Receiver<AgentMessage>) {
    let (tx, rx) = mpsc::channel(4096);
    jobs.set_log_acks(false);
    jobs.attach(&tx);
    jobs.send_stored_results().await;
    jobs.set_log_acks(true);
    jobs.resend_logs().await;
    (tx, rx)
}

/// FR-LG-07, AC-PR-07: the stream drops in the middle of a job. After the reconnect the control
/// plane has every line once and in order, and the acknowledged data leaves the disk.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_stream_drop_mid_job_loses_no_line_and_repeats_none() {
    let dir = directory("spool-drop-agent");
    let repo = directory("spool-drop-repo");
    let script = format!(
        "{}#sleep 3000\n{}",
        lines_script("first", 60),
        lines_script("second", 60)
    );
    script_stdout(&repo, "backup", &script);
    let jobs = program_jobs(&dir);

    let (tx, mut rx) = mpsc::channel(4096);
    jobs.attach(&tx);
    jobs.set_log_acks(true);
    start_job(&jobs, &repo, "job-drop").await;
    let mut stored = StoredLog::default();
    for _ in 0..100 {
        drain(&mut rx, &mut stored);
        if stored.lines().len() >= 60 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(stored.lines().len(), 60, "the first lines arrive live");
    jobs.log_ack(&stored.ack("job-drop"));
    // The stream drops: every later send fails, and the job runs on.
    drop(rx);
    drop(tx);
    wait_for_result(&dir).await;

    let (_tx, mut rx) = reconnect(&jobs).await;
    drain(&mut rx, &mut stored);
    let expected: Vec<String> = (1..=60)
        .map(|n| format!("first-{n:04}"))
        .chain((1..=60).map(|n| format!("second-{n:04}")))
        .collect();
    let lines: Vec<String> = stored.lines().into_iter().map(str::to_owned).collect();
    assert_eq!(lines, expected);

    jobs.log_ack(&stored.ack("job-drop"));
    assert_eq!(spool_files(&dir), 0, "an acknowledged log leaves the disk");
}

/// FR-LG-07: the agent restarts and the control plane never saw the log. The new process sends the
/// spool after the greeting.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_restart_sends_the_unacknowledged_spool() {
    let dir = directory("spool-restart-agent");
    let repo = directory("spool-restart-repo");
    script_stdout(&repo, "backup", &lines_script("line", 40));
    let jobs = program_jobs(&dir);
    start_job_then_lose_the_stream(&jobs, &repo, "job-restart").await;
    wait_for_result(&dir).await;
    assert_eq!(spool_files(&dir), 1);
    drop(jobs);

    let restarted = program_jobs(&dir);
    let (_tx, mut rx) = reconnect(&restarted).await;
    let mut stored = StoredLog::default();
    drain(&mut rx, &mut stored);
    assert_eq!(stored.lines().len(), 40);
    assert_eq!(stored.lines()[0], "line-0001");
    assert_eq!(stored.lines()[39], "line-0040");
    // A second connection sends the same chunks again until the control plane acknowledges them.
    let (_tx2, mut rx2) = reconnect(&restarted).await;
    let mut again = StoredLog::default();
    drain(&mut rx2, &mut again);
    assert_eq!(again.lines().len(), 40);

    restarted.log_ack(&stored.ack("job-restart"));
    assert_eq!(spool_files(&dir), 0);
}

/// FR-LG-07: the spool is full while the control plane is away. The log says how many lines it
/// lost instead of leaving a silent gap.
#[tokio::test(flavor = "multi_thread")]
async fn ac_lg_05_a_full_spool_leaves_a_marker_with_the_count() {
    let dir = directory("spool-cap-agent");
    let repo = directory("spool-cap-repo");
    let total = 3_000;
    script_stdout(
        &repo,
        "backup",
        &lines_script("line-with-some-padding-to-fill", total),
    );
    let jobs = program_jobs(&dir).with_log_spool(LogSpool::open(&dir, 8 * 1024));
    start_job_then_lose_the_stream(&jobs, &repo, "job-cap").await;
    wait_for_result(&dir).await;

    let (_tx, mut rx) = reconnect(&jobs).await;
    let mut stored = StoredLog::default();
    drain(&mut rx, &mut stored);
    let lines = stored.lines();
    let lost: usize = lines
        .iter()
        .filter(|line| {
            line.ends_with("log lines were lost while the control plane was unreachable")
        })
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

/// FR-LG-07, FR-AG-10: a control plane that reads nothing does not delay a cancel.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_10_a_stuck_stream_does_not_delay_a_cancel() {
    let dir = directory("spool-cancel-agent");
    let repo = directory("spool-cancel-repo");
    script_stdout(
        &repo,
        "backup",
        &format!("{}#sleep 30000\n", lines_script("busy", 400)),
    );
    let jobs = program_jobs(&dir);
    // A control plane that takes a log chunk and then stalls: every live send times out.
    let (tx, mut rx) = mpsc::channel::<AgentMessage>(2);
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if matches!(message.payload, Some(agent_message::Payload::LogChunk(_))) {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    });
    jobs.attach(&tx);
    jobs.set_log_acks(true);
    start_job(&jobs, &repo, "job-cancel").await;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let started = Instant::now();
    jobs.cancel("job-cancel");
    wait_for_result(&dir).await;
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
}
