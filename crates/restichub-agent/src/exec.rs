//! Hardening of the restic child process (FR-AG-06, ADR-0021, ADR-0031).
//!
//! The agent starts the bundled restic with a cleared environment and only the variables of the step
//! `CommandLine` (FR-AG-06). A secret reaches the child process only through the environment, never
//! through an argument (threat T9).
//!
//! The hash of the binary is computed through the opened file, and the child starts from that same
//! handle: on Linux the descriptor is kept open and the program is `/proc/self/fd/<fd>`, on Windows
//! the file is opened without `FILE_SHARE_WRITE` and `FILE_SHARE_DELETE` and the handle stays open
//! until the child starts. A binary that an attacker replaces between the check and the spawn is
//! therefore refused (threat T20).

use std::path::Path;
use std::process::{Child, Command};

use anyhow::{Context, Result};
use restichub_compiler::arg::CommandLine;

use crate::restic::{EXPECTED_SHA256, ResticBinary};

/// Builds the restic invocation with a cleared environment and only the step values (FR-AG-06).
///
/// The child process inherits no variable of the agent, so an ambient secret never reaches restic.
/// It holds the restic cache directory and, on Windows, the minimal variables that a child process
/// needs to load the system libraries and to write a temporary file.
#[must_use]
pub fn restic_command(program: &Path, line: &CommandLine, cache_dir: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    for (name, value) in line.env() {
        command.env(name, value);
    }
    command.env("RESTIC_CACHE_DIR", cache_dir);
    #[cfg(windows)]
    {
        // A child process needs `SystemRoot` to load the system libraries.
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        // Restic writes temporary files to `TEMP`/`TMP`; keep them in the agent cache directory.
        command.env("TEMP", cache_dir);
        command.env("TMP", cache_dir);
    }
    command.args(line.args());
    command
}

/// Re-checks the pinned hash and starts the child process (FR-AG-11, threat T20).
///
/// # Errors
///
/// Returns an error when the hash differs or the process cannot start.
pub fn spawn_restic(restic: &ResticBinary, line: &CommandLine) -> Result<Child> {
    spawn_verified(
        restic.path(),
        EXPECTED_SHA256,
        line,
        &crate::paths::cache_dir(),
    )
}

/// Opens the binary, checks the hash through the handle and starts the child from it (threat T20).
fn spawn_verified(
    path: &Path,
    expected_hex: &str,
    line: &CommandLine,
    cache: &Path,
) -> Result<Child> {
    let file = open_for_exec(path)?;
    let digest = digest_of(&file)?;
    crate::restic::verify_digest(&digest, expected_hex)
        .with_context(|| format!("the bundled restic at {}", path.display()))?;
    // The cache directory is created after the check, so an unverified binary never creates one.
    crate::paths::create_protected_dir(cache)?;
    let child = start(&file, path, line, cache)?;
    // The handle stays open until the child has started, so the checked inode is the one that runs.
    drop(file);
    Ok(child)
}

/// Reads the whole file through the handle and returns its SHA-256.
fn digest_of(file: &std::fs::File) -> Result<Vec<u8>> {
    use sha2::{Digest as _, Sha256};
    use std::io::Read as _;
    let mut reader = file;
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .context("cannot read the restic binary")?;
    Ok(Sha256::digest(&bytes).to_vec())
}

/// Opens the binary for the hash check and the start (threat T20).
#[cfg(target_os = "linux")]
fn open_for_exec(path: &Path) -> Result<std::fs::File> {
    std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))
}

/// Keeps the checked descriptor open across the exec, so the child runs the checked inode.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn start(file: &std::fs::File, _path: &Path, line: &CommandLine, cache: &Path) -> Result<Child> {
    use std::os::fd::AsRawFd as _;
    let fd = file.as_raw_fd();
    // `File::open` sets the close-on-exec flag. Clear it, so the descriptor survives the exec and
    // the child resolves `/proc/self/fd/<fd>` to the checked file (threat T20).
    // SAFETY: `fd` is an open descriptor that this function owns.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == -1 {
        return Err(std::io::Error::last_os_error()).context("cannot keep the restic handle open");
    }
    let program = std::path::PathBuf::from(format!("/proc/self/fd/{fd}"));
    restic_command(&program, line, cache)
        .spawn()
        .context("cannot start restic")
}

/// Opens the binary without `FILE_SHARE_WRITE` and `FILE_SHARE_DELETE` (threat T20).
#[cfg(windows)]
#[allow(unsafe_code)]
fn open_for_exec(path: &Path) -> Result<std::fs::File> {
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::FromRawHandle as _;
    use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, OPEN_EXISTING};
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is a NUL-terminated wide string; the handle is owned by the returned `File`.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot open {}", path.display()));
    }
    // SAFETY: `handle` is a valid file handle from `CreateFileW`.
    Ok(unsafe { std::fs::File::from_raw_handle(handle) })
}

/// Runs the child from the path while the pinned handle stays open (threat T20).
#[cfg(windows)]
fn start(_file: &std::fs::File, path: &Path, line: &CommandLine, cache: &Path) -> Result<Child> {
    restic_command(path, line, cache)
        .spawn()
        .context("cannot start restic")
}

/// Opens the binary for the hash check. macOS and other systems have no handle pin.
#[cfg(not(any(target_os = "linux", windows)))]
fn open_for_exec(path: &Path) -> Result<std::fs::File> {
    std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))
}

/// Runs the child from the path on a system without a handle pin.
#[cfg(not(any(target_os = "linux", windows)))]
fn start(_file: &std::fs::File, path: &Path, line: &CommandLine, cache: &Path) -> Result<Child> {
    restic_command(path, line, cache)
        .spawn()
        .context("cannot start restic")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    use restichub_compiler::ValidatedJob;
    use restichub_compiler::arg::build_command;
    use restichub_proto::agent::v1::{Job, RepositoryAccess, Step, UnlockStep, step};

    use super::restic_command;
    #[cfg(unix)]
    use super::spawn_verified;

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

    #[cfg(unix)]
    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "restichub-agent-exec-{name}-{}",
            std::process::id()
        ))
    }

    /// FR-AG-06: the child process holds only the step environment and no argument carries a secret.
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
    #[cfg(unix)]
    #[test]
    fn l2_the_child_starts_from_the_checked_handle() {
        use sha2::Digest as _;
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let script = temp_path("script");
        let mut file = std::fs::File::create(&script).expect("cannot create the script");
        file.write_all(b"#!/bin/sh\nexit 0\n")
            .expect("cannot write the script");
        drop(file);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("cannot make the script executable");

        let bytes = std::fs::read(&script).expect("cannot read the script");
        let expected = hex(&sha2::Sha256::digest(&bytes));

        let cache = temp_path("cache");
        std::fs::create_dir_all(&cache).expect("cannot create the cache");
        let line = command_line();
        let mut child = spawn_verified(&script, &expected, &line, &cache).expect("cannot start");
        let status = child.wait().expect("cannot wait");
        assert!(status.success(), "the child must run the checked file");
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// A wrong hash refuses the start, so the child never runs (threat T20).
    #[cfg(unix)]
    #[test]
    fn l2_a_wrong_hash_refuses_the_start() {
        use std::io::Write as _;
        let script = temp_path("wrong");
        let mut file = std::fs::File::create(&script).expect("cannot create the script");
        file.write_all(b"not a restic binary")
            .expect("cannot write");
        drop(file);
        let cache = temp_path("wrong-cache");
        std::fs::create_dir_all(&cache).expect("cannot create the cache");
        let line = command_line();
        let error = spawn_verified(&script, &"0".repeat(64), &line, &cache)
            .expect_err("a wrong hash must refuse the start");
        assert!(
            format!("{error:#}").contains("does not match the pinned SHA-256"),
            "{error:#}"
        );
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// The production entry refuses a binary that is not next to the agent (FR-AG-11).
    #[test]
    fn fr_ag_11_spawn_restic_refuses_a_missing_binary() {
        let restic = crate::restic::ResticBinary::locate().expect("cannot locate the restic path");
        let line = command_line();
        assert!(
            super::spawn_restic(&restic, &line).is_err(),
            "a missing binary must refuse the spawn"
        );
    }

    #[cfg(unix)]
    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(text, "{byte:02x}");
        }
        text
    }
}
