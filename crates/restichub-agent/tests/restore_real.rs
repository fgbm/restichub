//! A restore through the agent runner against the real restic and a local repository (AC-RS-03,
//! AC-RS-04, FR-RS-03, FR-RS-04).
//!
//! One agent writes the backup. A second agent, with its own directory and journal, restores the
//! snapshot to another directory, the way a restore to a different host runs. The tests skip with a
//! message when no restic runs on the machine, unless `RESTICHUB_REQUIRE_RESTIC` asks them to fail.

#![allow(clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::*;
use restichub_agent::jobs::Jobs;
#[cfg(unix)]
use restichub_proto::agent::v1::NodeType;
use restichub_proto::agent::v1::{
    JobStatus, OverwriteMode, RestoreStep, StepStatus, step, step_result::Output,
};

const PASSWORD: &str = "CANARY-restore-real-password";

struct Source {
    restic: PathBuf,
    repository: PathBuf,
    data: PathBuf,
    agent_a: PathBuf,
}

fn directory(name: &str) -> PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("cannot create the parent");
    std::fs::write(path, bytes).expect("cannot write the file");
}

/// Initializes a repository and backs up a small tree through the agent of the source host.
async fn backed_up(name: &str, files: &[(&str, &[u8])]) -> Option<(Source, String)> {
    backed_up_with(name, files, &[]).await
}

/// Initializes a repository and backs up a small tree of files and symbolic links through the agent
/// of the source host.
async fn backed_up_with(
    name: &str,
    files: &[(&str, &[u8])],
    links: &[(&str, &str)],
) -> Option<(Source, String)> {
    let restic = real_restic_or_skip()?;
    let source = Source {
        restic,
        repository: directory(&format!("{name}-repo")),
        data: directory(&format!("{name}-data")),
        agent_a: directory(&format!("{name}-agent-a")),
    };
    for (relative, bytes) in files {
        write(&source.data.join(relative), bytes);
    }
    #[cfg(unix)]
    for (relative, target) in links {
        std::os::unix::fs::symlink(target, source.data.join(relative))
            .expect("cannot create the link");
    }
    #[cfg(not(unix))]
    let _ = links;
    let access = access("repo-1", &source.repository, PASSWORD);
    let data = source.data.to_string_lossy().into_owned();
    let jobs = real_jobs(&source.agent_a, &source.restic);
    let backup = job(
        "job-backup",
        vec![access.clone()],
        vec![
            test_access("repo-1"),
            init("repo-1"),
            backup_step("repo-1", &[&data]),
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
    let Some(Output::SnapshotList(list)) = result.step("snapshots").output.clone() else {
        panic!("the snapshots step must give a list");
    };
    let snapshot = list.snapshots.first().expect("one snapshot").id.clone();
    Some((source, snapshot))
}

/// Runs one restore on a second agent, which has its own directory and journal.
async fn restore_on_agent_b(
    source: &Source,
    snapshot: &str,
    restore: RestoreStep,
    id: &str,
) -> (Collected, PathBuf) {
    let agent_b = directory(&format!("{id}-agent-b"));
    let jobs: Jobs = real_jobs(&agent_b, &source.restic);
    let job = job(
        id,
        vec![access("repo-1", &source.repository, PASSWORD)],
        vec![make_step(
            "restore",
            step::Kind::Restore(RestoreStep {
                snapshot_id: snapshot.to_owned(),
                ..restore
            }),
        )],
    );
    (run(&jobs, &job).await, agent_b)
}

/// Every file under `root`, by the path inside the restored copy of `data`, with its SHA-256.
fn tree(root: &Path, data: &Path) -> BTreeMap<String, String> {
    let name = data.file_name().expect("a name");
    let mut files = BTreeMap::new();
    let mut base = None;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry.expect("an entry").path();
            if entry.is_dir() {
                if entry.file_name() == Some(name) {
                    base = Some(entry.clone());
                }
                pending.push(entry);
            }
        }
    }
    let Some(base) = base else {
        return files;
    };
    let mut pending = vec![base.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("a directory") {
            let entry = entry.expect("an entry").path();
            if entry.is_dir() {
                pending.push(entry);
            } else {
                let relative = entry
                    .strip_prefix(&base)
                    .expect("inside the base")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, sha256_hex(&entry));
            }
        }
    }
    files
}

fn contains_canary(path: &Path) -> bool {
    let mut pending = vec![path.to_path_buf()];
    while let Some(entry) = pending.pop() {
        if entry.is_dir() {
            for child in std::fs::read_dir(&entry).expect("a directory") {
                pending.push(child.expect("an entry").path());
            }
        } else if std::fs::read(&entry).is_ok_and(|bytes| {
            bytes
                .windows(PASSWORD.len())
                .any(|w| w == PASSWORD.as_bytes())
        }) {
            return true;
        }
    }
    false
}

/// AC-RS-03: the files of agent B agree with the source by SHA-256, and agent B keeps no canary.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_03_a_restore_on_another_agent_gives_equal_hashes_and_no_canary() {
    let big: Vec<u8> = (0..300_000_u32).map(|n| (n * 31 % 251) as u8).collect();
    let Some((source, snapshot)) = backed_up(
        "ac-rs-03",
        &[
            ("a.txt", b"first file"),
            ("sub/big.bin", &big),
            ("sub/deep/c.txt", b"third file"),
            ("empty.txt", b""),
        ],
    )
    .await
    else {
        return;
    };
    let before = tree_of_source(&source);

    let target = directory("ac-rs-03-target");
    let (result, agent_b) = restore_on_agent_b(
        &source,
        &snapshot,
        RestoreStep {
            verify: true,
            ..restore_args("repo-1", &snapshot, &target)
        },
        "job-restore",
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    let Some(Output::RestoreSummary(summary)) = result.step("restore").output.clone() else {
        panic!(
            "the restore must give a typed summary: {}",
            result.log_text()
        );
    };
    assert!(!summary.dry_run);
    assert!(
        summary.files_restored >= 4,
        "restic counts the files and their directories"
    );
    assert_eq!(summary.total_bytes, big.len() as u64 + 20);

    let after = tree(&target, &source.data);
    assert_eq!(after, before, "the SHA-256 of every file agrees");
    assert_eq!(after.len(), 4);

    assert!(!contains_canary(&target), "no canary in the restored files");
    assert!(
        !contains_canary(&agent_b),
        "no canary on agent B after the job"
    );
    assert!(
        !result.log_text().contains(PASSWORD),
        "no canary in the log"
    );
    assert!(
        !format!("{:?}", result.messages).contains(PASSWORD),
        "no canary in a message of the agent"
    );
}

fn tree_of_source(source: &Source) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut pending = vec![source.data.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("a directory") {
            let entry = entry.expect("an entry").path();
            if entry.is_dir() {
                pending.push(entry);
            } else {
                let relative = entry
                    .strip_prefix(&source.data)
                    .expect("inside the data")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, sha256_hex(&entry));
            }
        }
    }
    files
}

fn names(files: &BTreeMap<String, String>) -> Vec<&str> {
    files.keys().map(String::as_str).collect()
}

/// One row of the selection table: the name, the includes, the excludes and the restored files.
type SelectCase = (&'static str, Vec<String>, Vec<String>, Vec<&'static str>);

/// AC-RS-04: include and exclude select the files in the table.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_04_include_and_exclude_select_the_files_of_the_table() {
    let Some((source, snapshot)) = backed_up(
        "ac-rs-04-select",
        &[
            ("a.txt", b"a"),
            ("b.log", b"b"),
            ("sub/c.txt", b"c"),
            ("sub/d.log", b"d"),
        ],
    )
    .await
    else {
        return;
    };
    let sub = source.data.join("sub").to_string_lossy().into_owned();
    // Patterns by name work on every host. The path pattern needs a Unix path.
    let mut table: Vec<SelectCase> = vec![
        (
            "all",
            vec![],
            vec![],
            vec!["a.txt", "b.log", "sub/c.txt", "sub/d.log"],
        ),
        (
            "exclude logs",
            vec![],
            vec!["*.log".to_owned()],
            vec!["a.txt", "sub/c.txt"],
        ),
        (
            "include texts",
            vec!["*.txt".to_owned()],
            vec![],
            vec!["a.txt", "sub/c.txt"],
        ),
    ];
    if cfg!(unix) {
        table.push((
            "include a directory",
            vec![sub],
            vec![],
            vec!["sub/c.txt", "sub/d.log"],
        ));
    }
    for (index, (case, includes, excludes, expected)) in table.into_iter().enumerate() {
        let target = directory(&format!("ac-rs-04-select-{index}-target"));
        let (result, _) = restore_on_agent_b(
            &source,
            &snapshot,
            RestoreStep {
                includes,
                excludes,
                ..restore_args("repo-1", &snapshot, &target)
            },
            &format!("job-select-{index}"),
        )
        .await;
        assert_eq!(
            result.result().outcome,
            JobStatus::Succeeded as i32,
            "{case}: {}",
            result.log_text()
        );
        let files = tree(&target, &source.data);
        assert_eq!(names(&files), expected, "{case}");
    }
}

/// The pattern that matches one path only. The Restore screen builds the same pattern
/// (`escapeGlob` in `ui/src/lib/restore.ts`): restic reads a backslash as a separator on Windows,
/// so each special character stands in a one-character class.
#[cfg(unix)]
fn escape_glob(path: &str) -> String {
    let mut escaped = String::new();
    for character in path.chars() {
        match character {
            '*' | '?' | '[' => escaped.push_str(&format!("[{character}]")),
            '\\' => escaped.push_str("[\\\\]"),
            other => escaped.push(other),
        }
    }
    match escaped.strip_prefix('!') {
        Some(rest) => format!("[!]{rest}"),
        None => escaped,
    }
}

/// FR-RS-02: a path with a wildcard character that the user selects in the tree restores only
/// itself. Without the escape, `[ab].txt` would select `a.txt` and `b.txt`.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_04_a_path_with_wildcard_characters_restores_only_itself() {
    let Some((source, snapshot)) = backed_up(
        "ac-rs-04-glob",
        &[
            ("a.txt", b"a"),
            ("b.txt", b"b"),
            ("[ab].txt", b"class"),
            ("a*.txt", b"star"),
            ("w?.txt", b"mark"),
            ("c].txt", b"close"),
            ("back\\slash.txt", b"slash"),
        ],
    )
    .await
    else {
        return;
    };
    let data = source.data.to_string_lossy().into_owned();
    let table: [(&str, &str, &[&str]); 7] = [
        ("plain", "a.txt", &["a.txt"]),
        ("class", "[ab].txt", &["[ab].txt"]),
        ("star", "a*.txt", &["a*.txt"]),
        ("mark", "w?.txt", &["w?.txt"]),
        ("close", "c].txt", &["c].txt"]),
        // `tree` reads a backslash as a separator.
        ("backslash", "back\\slash.txt", &["back/slash.txt"]),
        ("unescaped class", "", &[]),
    ];
    for (index, (case, name, expected)) in table.into_iter().enumerate() {
        let target = directory(&format!("ac-rs-04-glob-{index}-target"));
        // The last row shows what the raw name does: it selects the two files of the class.
        let (pattern, expected): (String, Vec<&str>) = if name.is_empty() {
            (format!("{data}/[ab].txt"), vec!["a.txt", "b.txt"])
        } else {
            (escape_glob(&format!("{data}/{name}")), expected.to_vec())
        };
        let (result, _) = restore_on_agent_b(
            &source,
            &snapshot,
            RestoreStep {
                includes: vec![pattern],
                ..restore_args("repo-1", &snapshot, &target)
            },
            &format!("job-glob-{index}"),
        )
        .await;
        assert_eq!(
            result.result().outcome,
            JobStatus::Succeeded as i32,
            "{case}: {}",
            result.log_text()
        );
        let files = tree(&target, &source.data);
        assert_eq!(names(&files), expected, "{case}");
    }
}

fn set_modified(path: &Path, time: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("cannot open the file")
        .set_modified(time)
        .expect("cannot set the time");
}

fn tree_root(target: &Path, data: &Path) -> PathBuf {
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

/// AC-RS-04: the overwrite modes keep or replace a file that exists in the target.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_04_the_overwrite_modes_keep_or_replace_an_existing_file() {
    let Some((source, snapshot)) = backed_up("ac-rs-04-overwrite", &[("a.txt", b"SNAPSHOT")]).await
    else {
        return;
    };
    // A first restore shows where restic writes the file in the target.
    let layout = directory("ac-rs-04-overwrite-layout");
    restore_on_agent_b(
        &source,
        &snapshot,
        restore_args("repo-1", &snapshot, &layout),
        "job-layout",
    )
    .await;
    let relative = tree_root(&layout, &source.data)
        .strip_prefix(&layout)
        .expect("inside the layout")
        .to_path_buf();

    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
    let future = SystemTime::now() + Duration::from_secs(10 * 86_400);
    let cases = [
        ("always replaces", OverwriteMode::Always, old, "SNAPSHOT"),
        (
            "always replaces a newer file",
            OverwriteMode::Always,
            future,
            "SNAPSHOT",
        ),
        (
            "if-changed replaces a file that differs",
            OverwriteMode::IfChanged,
            old,
            "SNAPSHOT",
        ),
        (
            "if-newer replaces an older file",
            OverwriteMode::IfNewer,
            old,
            "SNAPSHOT",
        ),
        (
            "if-newer keeps a newer file",
            OverwriteMode::IfNewer,
            future,
            "LOCAL",
        ),
        (
            "never keeps an older file",
            OverwriteMode::Never,
            old,
            "LOCAL",
        ),
        (
            "never keeps a newer file",
            OverwriteMode::Never,
            future,
            "LOCAL",
        ),
    ];
    for (index, (case, mode, modified, expected)) in cases.into_iter().enumerate() {
        let target = directory(&format!("ac-rs-04-overwrite-{index}-target"));
        let planted = target.join(&relative).join("a.txt");
        write(&planted, b"LOCAL");
        set_modified(&planted, modified);
        let (result, _) = restore_on_agent_b(
            &source,
            &snapshot,
            RestoreStep {
                overwrite: mode as i32,
                ..restore_args("repo-1", &snapshot, &target)
            },
            &format!("job-overwrite-{index}"),
        )
        .await;
        assert_eq!(
            result.result().outcome,
            JobStatus::Succeeded as i32,
            "{case}: {}",
            result.log_text()
        );
        let content = std::fs::read_to_string(&planted).expect("the file");
        assert_eq!(content, expected, "{case}");
    }
}

/// AC-RS-04: a dry run changes nothing in the file system, also with the mode that replaces files.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_04_a_dry_run_changes_no_file() {
    let Some((source, snapshot)) = backed_up(
        "ac-rs-04-dry-run",
        &[("a.txt", b"SNAPSHOT"), ("sub/b.txt", b"second")],
    )
    .await
    else {
        return;
    };
    let layout = directory("ac-rs-04-dry-run-layout");
    restore_on_agent_b(
        &source,
        &snapshot,
        restore_args("repo-1", &snapshot, &layout),
        "job-layout",
    )
    .await;
    let relative = tree_root(&layout, &source.data)
        .strip_prefix(&layout)
        .expect("inside the layout")
        .to_path_buf();

    // The target holds one file. A dry run with `always` must neither replace it nor add a file.
    let target = directory("ac-rs-04-dry-run-target");
    let planted = target.join(&relative).join("a.txt");
    write(&planted, b"LOCAL");
    let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
    set_modified(&planted, stamp);
    let (result, _) = restore_on_agent_b(
        &source,
        &snapshot,
        RestoreStep {
            dry_run: true,
            overwrite: OverwriteMode::Always as i32,
            ..restore_args("repo-1", &snapshot, &target)
        },
        "job-dry-run",
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    assert_eq!(result.step("restore").status, StepStatus::Success as i32);
    let Some(Output::RestoreSummary(summary)) = result.step("restore").output.clone() else {
        panic!("the dry run must give a summary: {}", result.log_text());
    };
    assert!(summary.dry_run);
    assert_eq!(
        std::fs::read_to_string(&planted).expect("the file"),
        "LOCAL"
    );
    assert_eq!(
        std::fs::metadata(&planted)
            .expect("metadata")
            .modified()
            .expect("time"),
        stamp
    );
    let files = tree(&target, &source.data);
    assert_eq!(names(&files), vec!["a.txt"], "the dry run added no file");

    // A target that does not exist stays without files.
    let missing = directory("ac-rs-04-dry-run-missing").join("never");
    let (result, _) = restore_on_agent_b(
        &source,
        &snapshot,
        RestoreStep {
            dry_run: true,
            ..restore_args("repo-1", &snapshot, &missing)
        },
        "job-dry-run-missing",
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    assert!(!missing.exists() || tree(&missing, &source.data).is_empty());
}

/// FR-RS-02: the page of a directory of the snapshot gives the target of a symbolic link.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn fr_rs_02_a_symlink_page_gives_the_target_of_the_link() {
    let Some((source, snapshot)) =
        backed_up_with("ls-link", &[("docs/a.txt", b"a")], &[("current", "docs")]).await
    else {
        return;
    };
    let jobs = real_jobs(&source.agent_a, &source.restic);
    let path = source.data.to_string_lossy().into_owned();
    let job = job(
        "job-ls-link",
        vec![access("repo-1", &source.repository, PASSWORD)],
        vec![ls_step("repo-1", &snapshot, &path, 0, 100)],
    );
    let result = run(&jobs, &job).await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    let Some(Output::LsPage(page)) = result.step("ls").output.clone() else {
        panic!("the ls step must give a page: {}", result.log_text());
    };
    let link = page
        .nodes
        .iter()
        .find(|node| node.name == "current")
        .expect("the page holds the link");
    assert_eq!(
        NodeType::try_from(link.r#type).ok(),
        Some(NodeType::Symlink)
    );
    assert_eq!(link.link_target, "docs");
}
