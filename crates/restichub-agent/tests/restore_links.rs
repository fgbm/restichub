//! A restore into a target that already holds links to outside the target, with the real restic
//! (FR-RS-03, threat T31).
//!
//! An earlier restore or a local user can leave a link in the target where the snapshot holds a
//! directory or a file. Restic on Windows writes a file through such a link, so the agent refuses
//! the step before restic starts. The tests skip with a message when no restic runs on the
//! machine, unless `RESTICHUB_REQUIRE_RESTIC` asks them to fail.

#![allow(clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::*;
use restichub_proto::agent::v1::{Job, JobStatus, RestoreStep, StepStatus, step, step_result};

const PASSWORD: &str = "restore-links-password";

fn directory(name: &str) -> PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

fn restore_job(id: &str, repository: &Path, snapshot: &str, target: &Path) -> Job {
    job(
        id,
        vec![access("repo-1", repository, PASSWORD)],
        vec![make_step(
            "restore",
            step::Kind::Restore(RestoreStep {
                snapshot_id: snapshot.to_owned(),
                ..restore_args("repo-1", snapshot, target)
            }),
        )],
    )
}

/// Finds where restic put the copy of `data` below `target`.
fn restored_data(target: &Path, data: &Path) -> PathBuf {
    let name = data.file_name().expect("a name");
    let mut pending = vec![target.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("a directory") {
            let entry = entry.expect("an entry").path();
            if entry.is_dir() {
                if entry.file_name() == Some(name) {
                    return entry;
                }
                pending.push(entry);
            }
        }
    }
    panic!("the restored tree has no {}", name.to_string_lossy());
}

#[cfg(unix)]
fn link_dir(original: &Path, link: &Path) {
    std::os::unix::fs::symlink(original, link).expect("cannot create the link");
}

#[cfg(windows)]
fn link_dir(original: &Path, link: &Path) {
    std::os::windows::fs::symlink_dir(original, link).expect("cannot create the link");
}

#[cfg(unix)]
fn link_file(original: &Path, link: &Path) {
    std::os::unix::fs::symlink(original, link).expect("cannot create the link");
}

#[cfg(windows)]
fn link_file(original: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(original, link).expect("cannot create the link");
}

/// FR-RS-03: a link in the target does not carry the restore outside the restore root.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_a_link_in_the_target_does_not_carry_the_restore_outside_the_root() {
    let Some(restic) = real_restic_or_skip() else {
        return;
    };
    let repository = directory("restore-links-repo");
    let data = directory("restore-links-data");
    std::fs::create_dir_all(data.join("sub")).expect("cannot create the source");
    std::fs::write(data.join("sub").join("f.txt"), b"SNAPSHOT").expect("cannot write");
    std::fs::write(data.join("a.txt"), b"SNAPSHOT").expect("cannot write");
    let jobs = real_jobs(&directory("restore-links-agent-a"), &restic);
    let source = data.to_string_lossy().into_owned();
    let backup = job(
        "job-backup",
        vec![access("repo-1", &repository, PASSWORD)],
        vec![
            test_access("repo-1"),
            init("repo-1"),
            backup_step("repo-1", &[&source]),
            snapshots_step("repo-1"),
        ],
    );
    let result = run(&jobs, &backup).await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    let Some(step_result::Output::SnapshotList(list)) = result.step("snapshots").output.clone()
    else {
        panic!("the snapshots step must give a list");
    };
    let snapshot = list.snapshots.first().expect("one snapshot").id.clone();

    let restorer = real_jobs(&directory("restore-links-agent-b"), &restic);
    let layout = directory("restore-links-layout");
    run(
        &restorer,
        &restore_job("job-layout", &repository, &snapshot, &layout),
    )
    .await;
    let relative = restored_data(&layout, &data)
        .strip_prefix(&layout)
        .expect("inside the layout")
        .to_path_buf();

    // The outside directory and file stand for a file that the agent must protect.
    let outside = directory("restore-links-outside");
    let outside_file = outside.join("victim.txt");
    std::fs::write(&outside_file, b"OUTSIDE").expect("cannot write");
    for (link, original) in [("sub", &outside), ("a.txt", &outside_file)] {
        let target = directory(&format!("restore-links-target-{link}"));
        let planted = target.join(&relative);
        std::fs::create_dir_all(&planted).expect("cannot create the planted tree");
        if original.is_dir() {
            link_dir(original, &planted.join(link));
        } else {
            link_file(original, &planted.join(link));
        }

        let result = run(
            &restorer,
            &restore_job(&format!("job-link-{link}"), &repository, &snapshot, &target),
        )
        .await;
        assert_eq!(
            result.result().outcome,
            JobStatus::Failed as i32,
            "{link}: {}",
            result.log_text()
        );
        let step = result.step("restore");
        assert_eq!(step.status, StepStatus::Failed as i32, "{link}");
        let reason = step
            .error
            .as_ref()
            .and_then(|error| error.params.iter().find(|param| param.key == "reason"))
            .map(|param| param.value.clone())
            .unwrap_or_default();
        assert!(
            reason.contains("a link that leads out of the target"),
            "{link}: {reason}"
        );
        assert!(reason.contains(link), "{link}: {reason}");
        let names: Vec<String> = std::fs::read_dir(&outside)
            .expect("the outside directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec!["victim.txt".to_owned()],
            "{link}: a file went out"
        );
        assert_eq!(
            std::fs::read(&outside_file).expect("the outside file"),
            b"OUTSIDE",
            "{link}"
        );
    }

    // A link that stays in the target does not stop the restore.
    let target = directory("restore-links-target-inside");
    let planted = target.join(&relative);
    std::fs::create_dir_all(planted.join("elsewhere")).expect("cannot create the planted tree");
    link_dir(&planted.join("elsewhere"), &planted.join("sub"));
    let result = run(
        &restorer,
        &restore_job("job-link-inside", &repository, &snapshot, &target),
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    assert_eq!(
        std::fs::read(planted.join("a.txt")).expect("the restored file"),
        b"SNAPSHOT"
    );
}
