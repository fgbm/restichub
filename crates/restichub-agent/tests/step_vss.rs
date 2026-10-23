//! The VSS lines of a backup log against a scripted restic (FR-AG-08).
//!
//! With `--json` restic prints no VSS message on success and an `error` message on the standard
//! error when it cannot make a snapshot.

#![allow(clippy::expect_used)]

mod common;

use std::path::PathBuf;

use common::*;
use restichub_proto::agent::v1::{BackupStep, StepStatus, step};

const SUMMARY: &str = r#"{"message_type":"summary","files_new":3,"files_changed":0,"files_unmodified":0,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":0,"data_blobs":1,"tree_blobs":1,"data_added":10,"data_added_packed":10,"total_files_processed":3,"total_bytes_processed":30,"total_duration":0.5,"snapshot_id":"abcdef0123456789"}"#;

const METADATA_ERROR: &str = r#"{"message_type":"error","error":{"message":"incomplete metadata for C:\\: get EA failed while opening file handle for path \\\\?\\GLOBALROOT\\Device\\HarddiskVolumeShadowCopy12\\, with: Access is denied."},"during":"archival","item":"C:\\"}"#;

const FAILURE: &str = r#"{"message_type":"error","error":{"message":"failed to create snapshot for [d:\\]: VSS error: the volume is not supported"},"during":"archival","item":"d:\\"}"#;

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

    async fn run(&self, job_id: &str, use_fs_snapshot: bool) -> Collected {
        let step = make_step(
            "backup",
            step::Kind::Backup(BackupStep {
                use_fs_snapshot,
                ..backup("repo-1", &[r"C:\data", r"D:\more"])
            }),
        );
        let job = job(
            job_id,
            vec![access("repo-1", &self.path, "vss-password")],
            vec![step],
        );
        run(&self.jobs(), &job).await
    }

    fn jobs(&self) -> restichub_agent::jobs::Jobs {
        program_jobs(&self.dir)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// FR-AG-08: the log names each volume that restic made a snapshot of.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_the_log_states_a_vss_snapshot_for_each_volume() {
    let fixture = Fixture::new("vss-made");
    script_stdout(&fixture.path, "backup", &format!("{SUMMARY}\n"));
    let result = fixture.run("job-vss-made", true).await;
    assert_eq!(result.step("backup").status, StepStatus::Success as i32);
    let log = result.log_text();
    assert!(
        log.contains(r"VSS snapshot made for C:\ (restic reported no VSS error)"),
        "{log}"
    );
    assert!(
        log.contains(r"VSS snapshot made for D:\ (restic reported no VSS error)"),
        "{log}"
    );
}

/// FR-AG-08: a volume that restic could not snapshot gets a failure line, not a success line. A
/// VSS failure alone leaves the exit code 0 and the step green.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_the_log_states_a_vss_snapshot_that_restic_could_not_make() {
    let fixture = Fixture::new("vss-failed");
    script_stdout(&fixture.path, "backup", &format!("{SUMMARY}\n"));
    script_stderr(&fixture.path, "backup", &format!("{FAILURE}\n"));
    script_code(&fixture.path, "backup", 0);
    let result = fixture.run("job-vss-failed", true).await;
    assert_eq!(result.step("backup").status, StepStatus::Success as i32);
    let log = result.log_text();
    assert!(
        log.contains(
            r"VSS snapshot for D:\ failed: VSS error: the volume is not supported; restic reads the files of this volume without a snapshot"
        ),
        "{log}"
    );
    assert!(
        log.contains(r"VSS snapshot made for C:\ (restic reported no VSS error)"),
        "{log}"
    );
    assert!(!log.contains(r"VSS snapshot made for D:\"), "{log}");
}

/// FR-AG-08: another error of restic on a drive root is not a VSS failure, and the log claims no
/// snapshot for that volume.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_an_error_on_a_drive_root_is_not_confirmed_as_a_vss_snapshot() {
    let fixture = Fixture::new("vss-unconfirmed");
    script_stdout(&fixture.path, "backup", &format!("{SUMMARY}\n"));
    script_stderr(&fixture.path, "backup", &format!("{METADATA_ERROR}\n"));
    script_code(&fixture.path, "backup", 3);
    let result = fixture.run("job-vss-unconfirmed", true).await;
    assert_eq!(result.step("backup").status, StepStatus::Warning as i32);
    let log = result.log_text();
    assert!(
        log.contains(r"VSS snapshot for C:\ not confirmed: incomplete metadata for C:\: "),
        "{log}"
    );
    assert!(!log.contains("without a snapshot"), "{log}");
    assert!(!log.contains(r"VSS snapshot made for C:\"), "{log}");
    assert!(
        log.contains(r"VSS snapshot made for D:\ (restic reported no VSS error)"),
        "{log}"
    );
}

/// FR-AG-08: a volume whose paths restic left out is not read, so the log claims no snapshot of it.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_a_volume_with_a_skipped_path_has_no_vss_snapshot_line() {
    let fixture = Fixture::new("vss-skipped");
    script_stdout(&fixture.path, "backup", &format!("{SUMMARY}\n"));
    script_stderr(
        &fixture.path,
        "backup",
        "D:\\more does not exist, skipping\n",
    );
    script_code(&fixture.path, "backup", 3);
    let result = fixture.run("job-vss-skipped", true).await;
    assert_eq!(result.step("backup").status, StepStatus::Warning as i32);
    let log = result.log_text();
    assert!(
        log.contains(r"VSS snapshot made for C:\ (restic reported no VSS error)"),
        "{log}"
    );
    assert!(!log.contains(r"VSS snapshot made for D:\"), "{log}");
}

/// FR-AG-08: a backup that ends without a summary claims no snapshot.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_a_backup_without_a_summary_claims_no_vss_snapshot() {
    let fixture = Fixture::new("vss-no-summary");
    script_code(&fixture.path, "backup", 1);
    let result = fixture.run("job-vss-none", true).await;
    assert_eq!(result.step("backup").status, StepStatus::Failed as i32);
    assert!(!result.log_text().contains("VSS snapshot made"));
}

/// FR-AG-08: a backup that asks for no snapshot has no VSS line.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_a_backup_without_the_option_has_no_vss_line() {
    let fixture = Fixture::new("vss-off");
    script_stdout(&fixture.path, "backup", &format!("{SUMMARY}\n"));
    let result = fixture.run("job-vss-off", false).await;
    assert!(!result.log_text().contains("VSS"));
}

/// FR-AG-08: the agent reads the line that restic prints for a missing path. A change of the text
/// in a new restic must fail here, not make the log claim a snapshot of a volume that restic did not
/// read.
#[test]
fn fr_ag_08_the_real_restic_prints_the_skip_line_that_the_agent_reads() {
    let Some(restic) = real_restic_or_skip() else {
        return;
    };
    let repository = temp_config_dir("vss-skip-repo");
    let data = temp_config_dir("vss-skip-data");
    std::fs::create_dir_all(&data).expect("cannot create the data directory");
    std::fs::write(data.join("a.txt"), "file").expect("cannot write the file");
    let missing = data.join("missing");

    let restic_run = |args: &[&std::ffi::OsStr]| {
        std::process::Command::new(&restic)
            .env("RESTIC_PASSWORD", "CANARY-vss-skip")
            .env("RESTIC_REPOSITORY", &repository)
            .args(args)
            .output()
            .expect("cannot run restic")
    };
    let init = restic_run(&["init".as_ref()]);
    assert!(init.status.success(), "{init:?}");
    let backup = restic_run(&[
        "backup".as_ref(),
        "--json".as_ref(),
        data.as_os_str(),
        missing.as_os_str(),
    ]);
    // Exit code 3: restic made a snapshot and left a path out.
    assert_eq!(backup.status.code(), Some(3), "{backup:?}");
    let stderr = String::from_utf8_lossy(&backup.stderr);
    let expected = format!("{} does not exist, skipping", missing.display());
    assert!(
        stderr.lines().any(|line| line == expected),
        "expected the line {expected:?} in {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&repository);
    let _ = std::fs::remove_dir_all(&data);
}
