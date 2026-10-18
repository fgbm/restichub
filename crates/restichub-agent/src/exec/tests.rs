#![allow(clippy::expect_used)]

use std::ffi::{OsStr, OsString};
use std::path::Path;

use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_proto::agent::v1::{Job, RepositoryAccess, Step, UnlockStep, step};

use super::{restic_command, spawn_verified};

fn command_line() -> restichub_compiler::CommandLine {
    let job = Job {
        job_id: "job-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        repositories: vec![RepositoryAccess {
            repository_id: "repo-1".to_owned(),
            location: "s3:s3.example.com/bucket".to_owned(),
            password: "topsecret".to_owned(),
            ..RepositoryAccess::default()
        }],
        steps: vec![Step {
            step_id: "unlock".to_owned(),
            kind: Some(step::Kind::Unlock(UnlockStep {
                repository_id: "repo-1".to_owned(),
            })),
            ..Step::default()
        }],
        ..Job::default()
    };
    let validated = ValidatedJob::new(job).expect("the job must validate");
    let step = validated.job().steps[0].clone();
    build_command(&validated, &step).expect("the command must build")
}

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "restichub-agent-exec-{name}-{}",
        std::process::id()
    ))
}

/// Writes a fake restic that exits at once: a copy of `true` on Unix and of the test executable
/// on Windows, because Windows runs no script directly (threat T20).
#[cfg(unix)]
fn write_fake_binary(path: &Path) {
    std::fs::copy("/bin/true", path).expect("cannot copy the fake binary");
}

/// Writes a fake `restic.exe`: the test executable exits at once for an unknown test filter.
#[cfg(windows)]
fn write_fake_binary(path: &Path) {
    let exe = std::env::current_exe().expect("cannot find the test executable");
    std::fs::copy(exe, path).expect("cannot copy the fake binary");
}

/// FR-AG-06: the child process holds only the step environment, the host proxy settings and no
/// argument carries a secret.
#[test]
fn fr_ag_06_the_child_environment_is_cleared() {
    let line = command_line();
    let cache = Path::new("/var/cache/restichub-agent");
    let program = Path::new("/opt/restichub-agent/restic");
    let command = restic_command(program, &line, cache);

    assert_eq!(
        command.get_program(),
        OsStr::new("/opt/restichub-agent/restic")
    );
    let args: Vec<&OsStr> = command.get_args().collect();
    let expected_args: Vec<&OsStr> = line.args().iter().map(OsStr::new).collect();
    assert_eq!(args, expected_args);

    let mut env: Vec<(OsString, Option<OsString>)> = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(OsString::from)))
        .collect();
    let mut expected: Vec<(OsString, Option<OsString>)> = line
        .env()
        .iter()
        .map(|(name, value)| (OsString::from(name), Some(OsString::from(value))))
        .collect();
    expected.push((
        OsString::from("RESTIC_CACHE_DIR"),
        Some(OsString::from("/var/cache/restichub-agent")),
    ));
    for name in super::PROXY_ENV {
        if let Some(value) = std::env::var_os(name) {
            expected.push((OsString::from(name), Some(value)));
        }
    }
    #[cfg(windows)]
    {
        expected.push((
            OsString::from("TEMP"),
            Some(OsString::from("/var/cache/restichub-agent")),
        ));
        expected.push((
            OsString::from("TMP"),
            Some(OsString::from("/var/cache/restichub-agent")),
        ));
        if let Some(root) = std::env::var_os("SystemRoot") {
            expected.push((OsString::from("SystemRoot"), Some(root)));
        }
    }
    // The iterator order of `get_envs` is not the insertion order.
    env.sort();
    expected.sort();
    assert_eq!(env, expected);

    // No variable of the parent process is inherited.
    assert!(
        !command
            .get_envs()
            .any(|(name, _)| name == OsStr::new("PATH"))
    );
    assert!(
        !command
            .get_envs()
            .any(|(name, _)| name == OsStr::new("HOME"))
    );
}

/// Runs the child from the inode whose hash the agent checked.
#[test]
fn l2_the_child_starts_from_the_checked_handle() {
    use sha2::Digest as _;

    let dir = temp_path("pinned");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    let program = dir.join(crate::restic::RESTIC_FILE_NAME);
    write_fake_binary(&program);

    let bytes = std::fs::read(&program).expect("cannot read the fake binary");
    let expected = hex(&sha2::Sha256::digest(&bytes));

    // The verified spawn creates the cache directory; a pre-created one fails the Windows
    // access check (FR-AG-04).
    let cache = temp_path("pinned-cache");
    let line = command_line();
    let mut child = retry_text_file_busy(|| spawn_verified(&program, &expected, &line, &cache))
        .expect("cannot start");
    let status = child.wait().expect("cannot wait");
    assert!(status.success(), "the child must run the checked file");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
}

/// A wrong hash refuses the start, so the child never runs (threat T20).
#[test]
fn l2_a_wrong_hash_refuses_the_start() {
    use std::io::Write as _;
    let path = temp_path("wrong");
    let mut file = std::fs::File::create(&path).expect("cannot create the file");
    file.write_all(b"not a restic binary")
        .expect("cannot write");
    drop(file);
    let cache = temp_path("wrong-cache");
    let line = command_line();
    let error = spawn_verified(&path, &"0".repeat(64), &line, &cache)
        .expect_err("a wrong hash must refuse the start");
    assert!(
        format!("{error:#}").contains("does not match the recorded SHA-256"),
        "{error:#}"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&cache);
}

/// An update replaces the binary and the record under a running service. The next job spawn
/// reads the new hash, so no restart is needed (FR-AG-11, threat T20).
#[cfg(unix)]
#[test]
fn fr_ag_11_an_update_takes_effect_for_the_next_spawn() {
    use restichub_proto::agent::v1::ProcessPriority;
    use sha2::Digest as _;

    use crate::restic_release::VerifiedRelease;
    use crate::restic_store::Layout;

    let base = temp_path("update");
    let _ = std::fs::remove_dir_all(&base);
    let layout = Layout::new(base.join("bin"), base.join("config"));
    let release = |tail: &[u8]| {
        let mut binary = std::fs::read("/bin/true").expect("cannot read true");
        binary.extend_from_slice(tail);
        VerifiedRelease {
            version: "0.19.1".to_owned(),
            sha256: hex(&sha2::Sha256::digest(&binary)),
            binary,
        }
    };
    let probe = |_: &Path| Ok("0.19.1".to_owned());
    let first = layout
        .install(&release(b""), &probe)
        .expect("first install");
    let jobs = crate::jobs::Jobs::open(
        layout.binary_path(),
        first.sha256.clone(),
        base.join("cache"),
        &base.join("config"),
    )
    .expect("jobs")
    .with_record(layout.clone());
    let line = command_line();
    let spawn = |expected: &str| {
        retry_text_file_busy(|| {
            super::spawn_job_restic(
                &layout.binary_path(),
                expected,
                &line,
                &base.join("cache"),
                ProcessPriority::Unspecified,
            )
        })
    };
    spawn(&jobs.expected_sha256().expect("hash")).expect("the first binary runs");

    let second = layout.install(&release(b"update"), &probe).expect("update");
    assert_ne!(first.sha256, second.sha256);
    let Err(stale) = spawn(&first.sha256) else {
        panic!("the old hash must no longer match");
    };
    assert!(
        format!("{stale:#}").contains("an administrator runs `restichub-agent restic update`"),
        "{stale:#}"
    );
    assert_eq!(jobs.expected_sha256().expect("hash"), second.sha256);
    spawn(&jobs.expected_sha256().expect("hash")).expect("the updated binary runs");
    let _ = std::fs::remove_dir_all(&base);
}

/// The production entry refuses a binary that is missing (FR-AG-11).
#[test]
fn fr_ag_11_spawn_restic_refuses_a_missing_binary() {
    let restic = crate::restic::ResticBinary::new(
        temp_path("missing-restic").join(crate::restic::RESTIC_FILE_NAME),
        "0".repeat(64),
    );
    let line = command_line();
    assert!(
        super::spawn_restic(&restic, &line).is_err(),
        "a missing binary must refuse the spawn"
    );
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Retries a spawn that fails with `ETXTBSY`. A test writes the fake binary while a parallel
/// test forks; the forked child holds the write handle until its own `exec`, so the kernel
/// refuses to run the new file for that moment.
fn retry_text_file_busy<T>(mut spawn: impl FnMut() -> anyhow::Result<T>) -> anyhow::Result<T> {
    for _ in 0..100 {
        match spawn() {
            Err(error) if format!("{error:#}").contains("Text file busy") => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            other => return other,
        }
    }
    spawn()
}

/// Reports whether a process is alive (Linux only; the signal needs `libc`).
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only tests the process for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// FR-AG-10, AC-AG-07: a child that ignores the graceful signal still dies with its whole tree
/// after the forced kill. The fake child is a shell that ignores `SIGINT` and spawns a subshell
/// that ignores it too, so only a group kill stops the tree.
#[cfg(target_os = "linux")]
#[test]
fn fr_ag_10_the_forced_kill_stops_the_whole_tree() {
    use std::process::Command as StdCommand;
    use std::time::Duration;

    let dir = temp_path("tree");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    let marker = dir.join("child.pid");
    let script = "trap '' INT; ( trap '' INT; echo $$ > \"$1\"; while :; do sleep 1; done ) & wait";

    let mut command = StdCommand::new("sh");
    command.arg("-c").arg(script).arg("sh").arg(&marker);
    super::apply_process_group(&mut command);
    let child = command.spawn().expect("cannot start the fake tree");
    let mut job = super::JobChild::new(child).expect("cannot wrap the child");

    let child_pid = wait_for_pid(&marker);
    // The graceful signal does not stop a tree that ignores it.
    job.interrupt().expect("cannot interrupt");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        alive(child_pid),
        "the tree ignored SIGINT and must still run"
    );

    // The forced kill stops the whole tree, not only the direct child.
    job.kill_tree().expect("cannot kill the tree");
    for _ in 0..200 {
        if job.try_wait().expect("cannot wait").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!alive(child_pid), "the child of the tree must be killed");
    let _ = std::fs::remove_dir_all(&dir);
}

/// FR-AG-15: the low priority is relative to the agent and never passes the highest nice value.
#[cfg(target_os = "linux")]
#[test]
fn ac_ag_14_the_low_priority_is_relative_to_the_agent() {
    assert_eq!(super::lowered_nice(0), 10);
    assert_eq!(super::lowered_nice(5), 15);
    assert_eq!(super::lowered_nice(15), 19);
    assert_eq!(super::lowered_nice(19), 19);
    assert_eq!(super::lowered_nice(-5), 5);
}

/// AC-AG-07: the kernel kills a restic child when the thread that started it ends.
#[cfg(target_os = "linux")]
#[test]
fn ac_ag_07_a_child_dies_with_its_parent() {
    use std::process::Command as StdCommand;
    use std::time::Duration;

    let (sender, receiver) = std::sync::mpsc::channel();
    // The thread ends after it starts the child, as the agent process does when it dies.
    std::thread::spawn(move || {
        let mut command = StdCommand::new("sleep");
        command.arg("30");
        super::apply_process_group(&mut command);
        let child = command.spawn().expect("cannot start the child");
        sender.send(child).expect("cannot hand over the child");
    })
    .join()
    .expect("the starter thread panicked");
    let mut child = receiver.recv().expect("no child");

    let mut exited = false;
    for _ in 0..300 {
        if child.try_wait().expect("cannot wait").is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !exited {
        let _ = child.kill();
    }
    assert!(exited, "the child must die with its parent thread");
}

/// Reads the child pid that the fake tree writes, or panics after two seconds.
#[cfg(target_os = "linux")]
fn wait_for_pid(marker: &Path) -> i32 {
    use std::time::Duration;
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(marker)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the fake tree wrote no child pid");
}
