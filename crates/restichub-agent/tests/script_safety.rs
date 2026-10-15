//! The safety rules of the script steps: the search path, the journal and the cleanup phases
//! (AC-HK-01, FR-HK-08, ADR-0037).

#![allow(clippy::expect_used)]
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt as _;

use common::scripts::*;
use common::*;
use restichub_proto::agent::v1::{HookPhase, JobStatus, StepStatus, agent_message};

fn directory(host: &Host, name: &str, mode: u32) -> std::path::PathBuf {
    let dir = host.fx.work.join(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("mode");
    dir
}

/// AC-HK-01: a search path directory that another account can write never reaches a script, and
/// the step log names the dropped directory once for the job.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_01_a_user_writable_search_path_directory_is_not_in_the_script_environment() {
    let host = Host::new("script-path");
    host.enable();
    let good = directory(&host, "bin-good", 0o755);
    let bad = directory(&host, "bin-bad", 0o777);
    let path = std::env::join_paths([good.as_path(), bad.as_path()]).expect("join");
    let record = |name: &str| format!("echo \"$PATH\" > '{}'\n", host.marks().join(name).display());
    let job = host.job(
        "job-path",
        vec![
            simple("script-0", HookPhase::Before, &record("path-0"), vec![]),
            simple("script-1", HookPhase::Before, &record("path-1"), vec![]),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    let collected = run(&host.jobs_with_path(path), &job).await;
    for step_id in ["script-0", "script-1"] {
        assert_eq!(status_of(&collected, step_id), StepStatus::Success);
    }
    for name in ["path-0", "path-1"] {
        let seen = host.mark(name);
        let entries: Vec<_> = std::env::split_paths(seen.trim_end()).collect();
        assert_eq!(entries, vec![good.clone()], "{name}");
    }
    let log = collected.log_text();
    let bad = bad.display().to_string();
    assert_eq!(log.matches(&bad).count(), 1, "{log}");
}

fn seed_in_progress(host: &Host, job_id: &str) {
    use restichub_agent::journal::{FileJournal, JOURNAL_FILE};
    use restichub_proto::signing::JobJournal as _;
    let mut journal = FileJournal::load(host.fx.dir.join(JOURNAL_FILE)).expect("journal");
    journal.record_accepted(job_id, at(600));
    journal.persist().expect("persist");
}

/// AC-PR-07: a job that a restart interrupted and that holds a script gets a final result and its
/// script does not run again.
#[tokio::test(flavor = "multi_thread")]
async fn ac_pr_07_an_interrupted_job_with_a_script_is_not_run_again() {
    let host = Host::new("script-interrupted");
    host.enable();
    let counter = host.marks().join("counter");
    std::fs::write(&counter, "run\n").expect("the first run");
    let body = format!("echo run >> '{}'\n", counter.display());
    let job = host.job(
        "job-interrupted",
        vec![
            simple("script-0", HookPhase::Before, &body, vec![]),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    seed_in_progress(&host, "job-interrupted");
    let jobs = host.jobs();
    for _ in 0..2 {
        let collected = run(&jobs, &job).await;
        let result = collected.result();
        assert_eq!(result.outcome, JobStatus::Failed as i32);
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("interrupted")
        );
        assert!(!collected.messages.iter().any(|message| matches!(
            message.payload,
            Some(agent_message::Payload::JobRejected(_))
        )));
    }
    assert_eq!(host.mark("counter").lines().count(), 1);
    assert!(host.fx.restic_runs().is_empty());
}

/// AC-PR-07: a job of restic steps only still runs again after a restart.
#[tokio::test(flavor = "multi_thread")]
async fn ac_pr_07_an_interrupted_job_without_a_script_runs_again() {
    let host = Host::new("script-restart-plain");
    host.enable();
    let job = host.job("job-plain", vec![backup_step("repo-1", &["/srv"])]);
    seed_in_progress(&host, "job-plain");
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
    assert!(!host.fx.restic_runs().is_empty());
}

/// AC-HK-07: a redelivered job that finished returns its stored result after the host turned the
/// trust mode off.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_07_a_completed_job_returns_its_result_after_the_mode_is_disabled() {
    let host = Host::new("script-completed");
    host.enable();
    let job = host.job(
        "job-done",
        vec![simple("script-0", HookPhase::Before, "true\n", vec![])],
    );
    let jobs = host.jobs();
    let first = run(&jobs, &job).await;
    assert_eq!(first.result().outcome, JobStatus::Succeeded as i32);
    host.write_setting("server_scripts = disabled\n", 0o644);
    let again = run(&jobs, &job).await;
    assert_eq!(again.result().outcome, JobStatus::Succeeded as i32);
    assert!(!again.messages.iter().any(|message| matches!(
        message.payload,
        Some(agent_message::Payload::JobRejected(_))
    )));
}

/// FR-HK-08: a script that turns the mode off stops the next script of the same job.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_08_disabling_the_mode_in_a_before_script_stops_the_next_script() {
    let host = Host::new("script-self-disable");
    host.enable();
    let disable = format!(
        "printf 'server_scripts = disabled\\n' > '{}'\n",
        host.setting().display()
    );
    let job = host.job(
        "job-self-disable",
        vec![
            simple("script-0", HookPhase::Before, &disable, vec![]),
            simple(
                "script-1",
                HookPhase::Before,
                &mark_body(&host, "second"),
                vec![],
            ),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Success);
    assert_ne!(status_of(&collected, "script-1"), StepStatus::Success);
    assert_eq!(host.mark("order"), "");
    assert!(host.fx.restic_runs().is_empty());
}
