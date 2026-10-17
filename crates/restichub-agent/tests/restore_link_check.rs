//! The link check of a restore target against a scripted restic (FR-RS-03, FR-AG-10, threat T31).
//!
//! The scripted `fake-restic` example lists `/file-0000000`, `/file-0000001` and so on, so a test
//! plants an entry on a listed path. The tests turn the Windows rule on, so they run on Linux too.

#![allow(clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use restichub_proto::agent::v1::{Job, JobStatus, RestoreStep, StepResult, StepStatus, step};
use tokio::sync::mpsc;

const PASSWORD: &str = "restore-link-check-password";

struct Fixture {
    dir: PathBuf,
    repository: PathBuf,
    target: PathBuf,
    outside: PathBuf,
}

impl Fixture {
    fn new(name: &str, listed: u64) -> Self {
        let dir = temp_config_dir(&format!("{name}-agent"));
        let repository = temp_config_dir(&format!("{name}-repo"));
        let target = temp_config_dir(&format!("{name}-target"));
        let outside = temp_config_dir(&format!("{name}-outside"));
        for path in [&dir, &repository, &target, &outside] {
            std::fs::create_dir_all(path).expect("cannot create the directory");
        }
        std::fs::write(
            format!("{}.restichub-ls-count", repository.display()),
            listed.to_string(),
        )
        .expect("cannot write the count");
        std::fs::write(target.join("unrelated.txt"), b"x").expect("cannot write");
        std::fs::write(outside.join("victim.txt"), b"OUTSIDE").expect("cannot write");
        Self {
            dir,
            repository,
            target,
            outside,
        }
    }

    fn job(&self, id: &str, change: impl FnOnce(&mut RestoreStep)) -> Job {
        let mut restore = restore_args("repo-1", "0123abcd", &self.target);
        change(&mut restore);
        job(
            id,
            vec![access("repo-1", &self.repository, PASSWORD)],
            vec![make_step("restore", step::Kind::Restore(restore))],
        )
    }

    fn commands(&self) -> Vec<String> {
        recorded_arguments(&self.repository)
            .lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for path in [&self.dir, &self.repository, &self.target, &self.outside] {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[cfg(unix)]
fn link_file(original: &Path, link: &Path) {
    std::os::unix::fs::symlink(original, link).expect("cannot create the link");
}

#[cfg(windows)]
fn link_file(original: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(original, link).expect("cannot create the link");
}

fn reason(step: &StepResult) -> String {
    step.error
        .as_ref()
        .and_then(|error| error.params.iter().find(|param| param.key == "reason"))
        .map(|param| param.value.clone())
        .unwrap_or_default()
}

/// FR-RS-03: a link on a listed path stops the restore before restic writes.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_a_link_on_a_listed_path_stops_the_restore() {
    let fixture = Fixture::new("link-check-refused", 3);
    link_file(
        &fixture.outside.join("victim.txt"),
        &fixture.target.join("file-0000001"),
    );
    let jobs = program_jobs(&fixture.dir).with_restore_link_rule(true);
    let result = run(&jobs, &fixture.job("job-refused", |_| {})).await;
    let step = result.step("restore");
    assert_eq!(step.status, StepStatus::Failed as i32);
    let reason = reason(step);
    assert!(reason.contains("file-0000001"), "{reason}");
    assert!(reason.contains("leads out of the target"), "{reason}");
    assert_eq!(fixture.commands(), vec!["ls".to_owned()]);
}

/// FR-RS-03: a dry run writes nothing, so the link check does not run.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_a_dry_run_skips_the_link_check() {
    let fixture = Fixture::new("link-check-dry-run", 3);
    link_file(
        &fixture.outside.join("victim.txt"),
        &fixture.target.join("file-0000001"),
    );
    let jobs = program_jobs(&fixture.dir).with_restore_link_rule(true);
    let result = run(
        &jobs,
        &fixture.job("job-dry-run", |restore| restore.dry_run = true),
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    assert_eq!(fixture.commands(), vec!["restore".to_owned()]);
}

/// FR-RS-03: an include that leaves out the link lets the restore run.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_a_link_off_the_included_paths_does_not_stop_the_restore() {
    let fixture = Fixture::new("link-check-include", 3);
    link_file(
        &fixture.outside.join("victim.txt"),
        &fixture.target.join("file-0000001"),
    );
    let jobs = program_jobs(&fixture.dir).with_restore_link_rule(true);
    let result = run(
        &jobs,
        &fixture.job("job-include", |restore| {
            restore.includes = vec!["/file-0000002".to_owned()];
        }),
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    assert_eq!(
        fixture.commands(),
        vec!["ls".to_owned(), "restore".to_owned()]
    );
}

/// FR-RS-03: a restore that writes more entries than the check takes is refused with a clear
/// reason.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_the_link_check_has_an_entry_limit() {
    let fixture = Fixture::new("link-check-limit", 1_000_001);
    let jobs = program_jobs(&fixture.dir).with_restore_link_rule(true);
    let result = run(&jobs, &fixture.job("job-limit", |_| {})).await;
    let step = result.step("restore");
    assert_eq!(step.status, StepStatus::Failed as i32);
    let reason = reason(step);
    assert!(reason.contains("narrow the includes"), "{reason}");
    assert_eq!(fixture.commands(), vec!["ls".to_owned()]);
}

/// FR-AG-10: a cancel during the link check ends the step as cancelled, and restic never restores.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_10_a_cancel_during_the_link_check_cancels_the_step() {
    let fixture = Fixture::new("link-check-cancel", 3);
    script_sleep(&fixture.repository, 20_000);
    let jobs = program_jobs_with_grace(&fixture.dir, Duration::from_millis(200))
        .with_restore_link_rule(true);
    let (tx, mut rx) = mpsc::channel(256);
    jobs.attach(&tx);
    jobs.greeted(false);
    jobs.handle(
        signed(&fixture.job("job-cancel", |_| {})),
        &ring(),
        AGENT,
        TENANT,
    )
    .await;
    assert!(
        wait_for_run(&fixture.repository, "ls").await,
        "the listing must start"
    );
    let cancelled_at = std::time::Instant::now();
    jobs.cancel("job-cancel");
    let result = collect(&mut rx).await;
    assert!(cancelled_at.elapsed() < Duration::from_secs(10));
    assert_eq!(result.result().outcome, JobStatus::Cancelled as i32);
    assert_eq!(result.step("restore").status, StepStatus::Cancelled as i32);
    assert_eq!(fixture.commands(), vec!["ls".to_owned()]);
}

/// AC-AG-05: a failed listing keeps the restic exit code that names the cause.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_05_a_failed_listing_keeps_the_restic_exit_code() {
    for (code, status) in [
        (10, StepStatus::ConfigError),
        (11, StepStatus::Retry),
        (12, StepStatus::ConfigError),
    ] {
        let fixture = Fixture::new(&format!("link-check-exit-{code}"), 3);
        std::fs::write(
            format!("{}.restichub-code-ls", fixture.repository.display()),
            code.to_string(),
        )
        .expect("cannot write the code");
        let jobs = program_jobs(&fixture.dir).with_restore_link_rule(true);
        let result = run(&jobs, &fixture.job(&format!("job-exit-{code}"), |_| {})).await;
        let step = result.step("restore");
        assert_eq!(step.status, status as i32, "code {code}");
        assert_eq!(step.exit_code, code, "code {code}");
        assert_eq!(fixture.commands(), vec!["ls".to_owned()]);
    }
}
