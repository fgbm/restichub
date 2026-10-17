//! A restore into a target that already holds links to outside the target, with the real restic
//! (FR-RS-03, threat T31).
//!
//! An earlier restore or a local user can leave a link in the target where the snapshot holds a
//! directory or a file. Restic on Windows writes a file through such a link, so the agent refuses
//! the step there before restic starts. Restic on Linux replaces the link, so Linux has no such
//! rule; the tests turn the rule on to cover it on both systems. The tests skip with a message when
//! no restic runs on the machine, unless `RESTICHUB_REQUIRE_RESTIC` asks them to fail.

#![allow(clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::*;
use restichub_agent::jobs::Jobs;
use restichub_proto::agent::v1::{Job, JobStatus, RestoreStep, StepStatus, step, step_result};

const PASSWORD: &str = "restore-links-password";

fn directory(name: &str) -> PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
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
fn link(original: &Path, link: &Path) {
    std::os::unix::fs::symlink(original, link).expect("cannot create the link");
}

#[cfg(windows)]
fn link(original: &Path, link: &Path) {
    if original.is_dir() {
        std::os::windows::fs::symlink_dir(original, link).expect("cannot create the link");
    } else {
        std::os::windows::fs::symlink_file(original, link).expect("cannot create the link");
    }
}

/// One backup of `data/{a.txt, other.txt, sub/f.txt}` and where restic puts `data` below a target.
struct Snapshot {
    restic: PathBuf,
    repository: PathBuf,
    data: PathBuf,
    id: String,
    /// The path of `data` below a target, as restic restores it.
    relative: PathBuf,
}

impl Snapshot {
    async fn take(name: &str) -> Option<Self> {
        let restic = real_restic_or_skip()?;
        let repository = directory(&format!("{name}-repo"));
        let data = directory(&format!("{name}-data"));
        std::fs::create_dir_all(data.join("sub")).expect("cannot create the source");
        std::fs::write(data.join("sub").join("f.txt"), b"SNAPSHOT").expect("cannot write");
        std::fs::write(data.join("a.txt"), b"SNAPSHOT").expect("cannot write");
        std::fs::write(data.join("other.txt"), b"SNAPSHOT").expect("cannot write");
        let jobs = real_jobs(&directory(&format!("{name}-agent-a")), &restic);
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
        let id = list.snapshots.first().expect("one snapshot").id.clone();
        let mut snapshot = Self {
            restic,
            repository,
            data,
            id,
            relative: PathBuf::new(),
        };
        let layout = directory(&format!("{name}-layout"));
        let jobs = snapshot.jobs(&format!("{name}-layout-agent"));
        run(&jobs, &snapshot.restore("job-layout", &layout, |_| {})).await;
        snapshot.relative = restored_data(&layout, &snapshot.data)
            .strip_prefix(&layout)
            .expect("inside the layout")
            .to_path_buf();
        Some(snapshot)
    }

    fn jobs(&self, name: &str) -> Jobs {
        real_jobs(&directory(name), &self.restic)
    }

    fn restore(&self, id: &str, target: &Path, change: impl FnOnce(&mut RestoreStep)) -> Job {
        let mut restore = restore_args("repo-1", &self.id, target);
        change(&mut restore);
        job(
            id,
            vec![access("repo-1", &self.repository, PASSWORD)],
            vec![make_step("restore", step::Kind::Restore(restore))],
        )
    }

    /// A fresh directory with the parents of `data` in it; gives it and the planted `data`.
    fn target(&self, name: &str) -> (PathBuf, PathBuf) {
        let target = directory(name);
        let planted = target.join(&self.relative);
        std::fs::create_dir_all(&planted).expect("cannot create the planted tree");
        (target, planted)
    }

    fn include(&self, name: &str) -> String {
        self.data.join(name).to_string_lossy().into_owned()
    }
}

/// An outside directory with one file that the agent must protect.
fn outside(name: &str) -> (PathBuf, PathBuf) {
    let outside = directory(name);
    let victim = outside.join("victim.txt");
    std::fs::write(&victim, b"OUTSIDE").expect("cannot write");
    (outside, victim)
}

fn assert_untouched(outside: &Path, victim: &Path, case: &str) {
    let names: Vec<String> = std::fs::read_dir(outside)
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
        "{case}: a file went out"
    );
    assert_eq!(
        std::fs::read(victim).expect("the outside file"),
        b"OUTSIDE",
        "{case}"
    );
}

/// FR-RS-03: a link or a second name on a restored path, or on its ancestor, stops the step.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_a_link_on_a_restored_path_is_refused() {
    let Some(snapshot) = Snapshot::take("restore-links-refused").await else {
        return;
    };
    let jobs = snapshot
        .jobs("restore-links-refused-agent")
        .with_restore_link_rule(true);
    let (outside, victim) = outside("restore-links-refused-outside");
    let gone = outside.join("gone.txt");
    for case in ["file", "ancestor", "dangling", "hardlink"] {
        let (target, planted) = snapshot.target(&format!("restore-links-refused-{case}"));
        let name = match case {
            "file" => {
                link(&victim, &planted.join("a.txt"));
                "a.txt"
            }
            "ancestor" => {
                link(&outside, &planted.join("sub"));
                "sub"
            }
            "dangling" => {
                std::fs::write(&gone, b"x").expect("cannot write");
                link(&gone, &planted.join("a.txt"));
                std::fs::remove_file(&gone).expect("cannot remove");
                "a.txt"
            }
            _ => {
                std::fs::hard_link(&victim, planted.join("a.txt")).expect("cannot link");
                "a.txt"
            }
        };
        let result = run(
            &jobs,
            &snapshot.restore(&format!("job-{case}"), &target, |_| {}),
        )
        .await;
        assert_eq!(
            result.result().outcome,
            JobStatus::Failed as i32,
            "{case}: {}",
            result.log_text()
        );
        let step = result.step("restore");
        assert_eq!(step.status, StepStatus::Failed as i32, "{case}");
        let reason = step
            .error
            .as_ref()
            .and_then(|error| error.params.iter().find(|param| param.key == "reason"))
            .map(|param| param.value.clone())
            .unwrap_or_default();
        assert!(
            reason.contains("on a path that the restore writes"),
            "{case}: {reason}"
        );
        assert!(reason.contains(name), "{case}: {reason}");
        assert_untouched(&outside, &victim, case);
        let _ = std::fs::remove_dir_all(&target);
    }
}

/// FR-RS-03: a link that the restore does not write through, or that stays in the target, lets the
/// restore run.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_a_link_off_the_restored_paths_or_inside_the_target_does_not_block() {
    let Some(snapshot) = Snapshot::take("restore-links-pass").await else {
        return;
    };
    let jobs = snapshot
        .jobs("restore-links-pass-agent")
        .with_restore_link_rule(true);
    let (outside, victim) = outside("restore-links-pass-outside");
    // An include by path needs a Unix path, as in `restore_real.rs`.
    let cases: &[&str] = if cfg!(unix) {
        &["venv", "excluded-by-include", "inside", "self"]
    } else {
        &["venv", "inside", "self"]
    };
    for &case in cases {
        let (target, planted) = snapshot.target(&format!("restore-links-pass-{case}"));
        let mut includes = Vec::new();
        match case {
            // The snapshot has no `venv`, as with a virtual environment next to the data.
            "venv" => link(&outside, &planted.join("venv")),
            "excluded-by-include" => {
                link(&outside, &planted.join("sub"));
                includes.push(snapshot.include("a.txt"));
            }
            "inside" => {
                std::fs::create_dir_all(planted.join("elsewhere")).expect("cannot create");
                link(&planted.join("elsewhere"), &planted.join("sub"));
            }
            _ => link(&target, &planted.join("sub")),
        }
        let result = run(
            &jobs,
            &snapshot.restore(&format!("job-{case}"), &target, |restore| {
                restore.includes = includes;
            }),
        )
        .await;
        assert_eq!(
            result.result().outcome,
            JobStatus::Succeeded as i32,
            "{case}: {}",
            result.log_text()
        );
        assert_eq!(
            std::fs::read(planted.join("a.txt")).expect("the restored file"),
            b"SNAPSHOT",
            "{case}"
        );
        assert_untouched(&outside, &victim, case);
        let _ = std::fs::remove_dir_all(&target);
    }
}

/// FR-RS-03: restic on Linux replaces a link on a restored path with the node of the snapshot, so
/// the agent has no link rule there.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_03_restic_on_linux_replaces_a_link_on_a_restored_path() {
    let Some(snapshot) = Snapshot::take("restore-links-linux").await else {
        return;
    };
    let jobs = snapshot.jobs("restore-links-linux-agent");
    let (outside, victim) = outside("restore-links-linux-outside");
    let (target, planted) = snapshot.target("restore-links-linux-target");
    link(&victim, &planted.join("a.txt"));
    link(&outside, &planted.join("sub"));
    let result = run(&jobs, &snapshot.restore("job-linux", &target, |_| {})).await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    assert_untouched(&outside, &victim, "linux");
    let file = std::fs::symlink_metadata(planted.join("a.txt")).expect("the restored file");
    assert!(file.is_file(), "restic replaced the file link");
    let dir = std::fs::symlink_metadata(planted.join("sub")).expect("the restored directory");
    assert!(dir.is_dir(), "restic replaced the directory link");
    assert_eq!(
        std::fs::read(planted.join("sub").join("f.txt")).expect("the restored file"),
        b"SNAPSHOT"
    );
}
