//! The `ls` and `restore` steps against a scripted restic, without a control plane (FR-RS-02,
//! FR-RS-04, FR-PR-10, ADR-0021, AC-RS-02).
//!
//! The tests drive [`Jobs::handle`] directly. The scripted `fake-restic` example prints the JSON
//! lines of the test, or a generated listing of any length.

#![allow(clippy::expect_used)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::*;
use prost::Message as _;
use restichub_proto::agent::v1::{
    JobStatus, OverwriteMode, RejectReason, RestoreStep, StepStatus, agent_message, step,
    step_result::Output,
};
use tokio::sync::mpsc;

const PASSWORD: &str = "CANARY-restore-step-password";

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

    fn job(
        &self,
        id: &str,
        steps: Vec<restichub_proto::agent::v1::Step>,
    ) -> restichub_proto::agent::v1::Job {
        job(id, vec![access("repo-1", &self.path, PASSWORD)], steps)
    }

    fn argv(&self) -> Vec<String> {
        recorded_arguments(&self.path)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn list(&self, count: u64) {
        std::fs::write(
            format!("{}.restichub-ls-count", self.path.display()),
            count.to_string(),
        )
        .expect("cannot write the count");
    }
}

/// AC-RS-02: the first page of a listing with one million entries comes in 5 s or less and holds
/// the entries of the page only. The agent stops restic when the page is full.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_02_the_first_page_of_a_million_entries_is_small_and_fast() {
    let fixture = Fixture::new("ls-million");
    fixture.list(1_000_000);
    let jobs = program_jobs(&fixture.dir);

    let started = Instant::now();
    let result = run(
        &jobs,
        &fixture.job(
            "job-ls",
            vec![ls_step("repo-1", "latest", "/data", 0, 1_000)],
        ),
    )
    .await;
    let elapsed = started.elapsed();

    assert_eq!(result.result().outcome, JobStatus::Succeeded as i32);
    assert_eq!(result.step("ls").status, StepStatus::Success as i32);
    let Some(Output::LsPage(page)) = result.step("ls").output.clone() else {
        panic!("the ls step must give a page: {}", result.log_text());
    };
    assert_eq!(page.nodes.len(), 1_000);
    assert!(!page.last, "more entries follow");
    // The listed directory is not an entry of itself.
    assert_eq!(page.nodes[0].name, "file-0000000");
    assert_eq!(page.nodes[999].path, "/data/file-0000999");
    assert!(
        result.step("ls").encoded_len() < restichub_proto::MAX_STEP_RESULT_BYTES,
        "a page fits in a step result"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the first page took {elapsed:?}"
    );
    assert_eq!(
        fixture.argv(),
        vec!["ls --json -- latest /data".to_owned()],
        "the directory goes after --"
    );
}

/// AC-RS-02: the next pages load on request, and the last page says so.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_02_a_later_page_skips_the_earlier_entries_and_the_last_page_ends_the_listing() {
    let fixture = Fixture::new("ls-pages");
    fixture.list(2_500);
    let jobs = program_jobs(&fixture.dir);

    let page = |name: &'static str, offset: u64| {
        let fixture = &fixture;
        let jobs = &jobs;
        async move {
            let result = run(
                jobs,
                &fixture.job(
                    name,
                    vec![ls_step("repo-1", "latest", "/data", offset, 1_000)],
                ),
            )
            .await;
            let Some(Output::LsPage(page)) = result.step("ls").output.clone() else {
                panic!("the ls step must give a page: {}", result.log_text());
            };
            page
        }
    };
    let second = page("job-page-2", 1_000).await;
    assert_eq!(second.nodes.len(), 1_000);
    assert!(!second.last);
    assert_eq!(second.nodes[0].name, "file-0001000");
    let third = page("job-page-3", 2_000).await;
    assert_eq!(third.nodes.len(), 500);
    assert!(third.last, "the page holds the rest of the directory");
    assert_eq!(third.nodes[499].name, "file-0002499");
    let exact = page("job-page-exact", 1_500).await;
    assert_eq!(exact.nodes.len(), 1_000);
    assert!(
        exact.last,
        "a page that ends with the listing is the last page"
    );
}

/// FR-RS-02: the root is `/`, because restic lists the whole tree without a directory.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_02_the_root_listing_names_the_root_directory() {
    let fixture = Fixture::new("ls-root");
    let jobs = program_jobs(&fixture.dir);
    run(
        &jobs,
        &fixture.job("job-root", vec![ls_step("repo-1", "0123abcd", "", 0, 0)]),
    )
    .await;
    assert_eq!(fixture.argv(), vec!["ls --json -- 0123abcd /".to_owned()]);
}

/// FR-RS-04: restic prints a status and a summary; the agent shows the progress and keeps the
/// summary as typed output. The argument vector follows ADR-0021.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_04_a_restore_reports_progress_and_a_typed_summary() {
    let fixture = Fixture::new("restore-summary");
    let jobs = program_jobs(&fixture.dir);
    let target = temp_config_dir("restore-summary-target");
    script_stdout(
        &fixture.path,
        "restore",
        concat!(
            r#"{"message_type":"status","seconds_elapsed":2,"percent_done":0.5,"total_files":10,"files_restored":5,"total_bytes":2000,"bytes_restored":1000}"#,
            "\n#sleep 300\n",
            r#"{"message_type":"summary","seconds_elapsed":4,"total_files":10,"files_restored":9,"files_skipped":1,"total_bytes":2000,"bytes_restored":1800,"bytes_skipped":200}"#,
            "\n",
        ),
    );
    let restore = RestoreStep {
        excludes: vec!["*.log".to_owned()],
        dry_run: true,
        overwrite: OverwriteMode::IfNewer as i32,
        verify: true,
        ..restore_args("repo-1", "latest", &target)
    };
    let result = run(
        &jobs,
        &fixture.job(
            "job-restore",
            vec![make_step("restore", step::Kind::Restore(restore))],
        ),
    )
    .await;

    assert_eq!(result.result().outcome, JobStatus::Succeeded as i32);
    let Some(Output::RestoreSummary(summary)) = result.step("restore").output.clone() else {
        panic!(
            "the restore must give a typed summary: {}",
            result.log_text()
        );
    };
    assert_eq!(summary.total_files, 10);
    assert_eq!(summary.files_restored, 9);
    assert_eq!(summary.files_skipped, 1);
    assert_eq!(summary.bytes_restored, 1_800);
    assert!(
        summary.dry_run,
        "the summary says that the run was a dry run"
    );
    assert!(
        result
            .progress
            .iter()
            .all(|progress| progress.step_id == "restore"),
        "{:?}",
        result.progress
    );
    if let Some(progress) = result.progress.first() {
        assert_eq!(progress.files_done, 5);
        assert_eq!(progress.bytes_done, 1_000);
        assert_eq!(progress.total_bytes, 2_000);
    }
    assert_eq!(
        fixture.argv(),
        vec![format!(
            "restore --json --exclude=*.log --target={} --dry-run --overwrite=if-newer --verify -- latest",
            target.display()
        )]
    );
}

/// ADR-0021: a value from the job never becomes an option, and a bad value stops the job before
/// restic starts.
#[tokio::test(flavor = "multi_thread")]
async fn fr_pr_10_a_restore_value_never_becomes_an_option() {
    let fixture = Fixture::new("restore-injection");
    let jobs = program_jobs(&fixture.dir);
    let target = temp_config_dir("restore-injection-target");
    let hostile = "--password-command=touch /tmp/pwned";
    for (index, restore) in [
        RestoreStep {
            includes: vec![hostile.to_owned()],
            ..restore_args("repo-1", "latest", &target)
        },
        RestoreStep {
            excludes: vec!["-x".to_owned()],
            ..restore_args("repo-1", "latest", &target)
        },
    ]
    .into_iter()
    .enumerate()
    {
        run(
            &jobs,
            &fixture.job(
                &format!("job-hostile-{index}"),
                vec![make_step("restore", step::Kind::Restore(restore))],
            ),
        )
        .await;
    }
    let argv = fixture.argv();
    assert_eq!(argv.len(), 2);
    assert!(
        argv[0].contains(&format!("--include={hostile}")),
        "{}",
        argv[0]
    );
    assert!(argv[1].contains("--exclude=-x"), "{}", argv[1]);
    assert!(
        argv.iter().all(|line| line.ends_with("-- latest")),
        "{argv:?}"
    );

    // A snapshot identifier that is not hexadecimal, and a target that is not absolute, are
    // refused by the validation of the job.
    for (index, bad) in [
        RestoreStep {
            snapshot_id: "--dry-run".to_owned(),
            ..restore_args("repo-1", "latest", &target)
        },
        RestoreStep {
            target: "-rf".to_owned(),
            ..restore_args("repo-1", "latest", &target)
        },
        RestoreStep {
            target: "relative/dir".to_owned(),
            ..restore_args("repo-1", "latest", &target)
        },
        RestoreStep {
            includes: vec!["/srv".to_owned()],
            excludes: vec!["*.log".to_owned()],
            ..restore_args("repo-1", "latest", &target)
        },
    ]
    .into_iter()
    .enumerate()
    {
        let (tx, mut rx) = mpsc::channel(8);
        jobs.attach(&tx);
        let job = fixture.job(
            &format!("job-bad-{index}"),
            vec![make_step("restore", step::Kind::Restore(bad))],
        );
        jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
        let message = rx.recv().await.expect("the agent must answer");
        match message.payload {
            Some(agent_message::Payload::JobRejected(rejected)) => assert_eq!(
                RejectReason::try_from(rejected.reason).ok(),
                Some(RejectReason::Malformed)
            ),
            other => panic!("the job must be rejected: {other:?}"),
        }
    }
    assert_eq!(fixture.argv().len(), 2, "the refused jobs start no restic");
}
