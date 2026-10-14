//! Hardening of the restic child process (FR-AG-06, FR-AG-11, ADR-0021, ADR-0031).
//!
//! The agent starts the bundled restic with a cleared environment, the variables of the step
//! `CommandLine` and the proxy variables of the host (FR-AG-06). A secret reaches the child process
//! only through the environment, never through an argument (threat T9).
//!
//! The hash of the binary is computed through the opened file, and the child starts from that same
//! handle: on Linux the descriptor keeps its close-on-exec flag and the program is
//! `/proc/self/fd/<fd>`, which the kernel resolves to the checked file before it closes the
//! descriptor; on Windows the file is opened without `FILE_SHARE_WRITE` and `FILE_SHARE_DELETE` and
//! the handle stays open until the child starts. A binary that an attacker replaces between the
//! check and the spawn is therefore refused (threat T20).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};

use anyhow::{Context, Result};
use restichub_compiler::arg::CommandLine;

use crate::restic::{EXPECTED_SHA256, ResticBinary};

/// The proxy variables that a restic child inherits from the agent environment (FR-AG-06).
///
/// A repository behind a proxy uses them. They come from the agent environment only, never from a
/// step, so a job cannot redirect the traffic of a repository. Windows holds the environment names
/// case-insensitively, so the upper-case names cover the lower-case forms there.
#[cfg(windows)]
const PROXY_ENV: [&str; 3] = ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY"];

/// The proxy variables that a restic child inherits from the agent environment (FR-AG-06).
#[cfg(not(windows))]
const PROXY_ENV: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// Builds the restic invocation with a cleared environment and only the step values (FR-AG-06).
///
/// The child process inherits no variable of the agent except the proxy variables of the host, so
/// an ambient secret never reaches restic. It holds the restic cache directory, the host proxy
/// settings and, on Windows, the minimal variables that a child process needs to load the system
/// libraries and to write a temporary file.
#[must_use]
pub fn restic_command(program: &Path, line: &CommandLine, cache_dir: &Path) -> Command {
    restic_command_with(program, line.args(), line.env(), cache_dir)
}

/// Builds the restic invocation for a bare argument list (FR-AG-11).
fn restic_command_with(
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    cache_dir: &Path,
) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    for (name, value) in env {
        command.env(name, value);
    }
    command.env("RESTIC_CACHE_DIR", cache_dir);
    for name in PROXY_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
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
    command.args(args);
    apply_process_group(&mut command);
    command
}

/// Puts a restic child in its own process group, so a stop reaches its whole tree (FR-AG-10).
///
/// On Unix the child becomes the leader of a new group; the agent signals the group with a negative
/// pid. On Windows the child becomes a new process group and the agent sends it `CTRL_BREAK`.
fn apply_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
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

/// Runs the pinned restic with `version` and collects its output (FR-AG-11, threat T20).
///
/// The child runs the checked inode with the environment of a job (FR-AG-06), so the check and the
/// run share one reading of the binary.
///
/// # Errors
///
/// Returns an error when the hash differs or the process cannot start.
pub fn version_output(restic: &ResticBinary) -> Result<Output> {
    let (file, mut command) = verified_command(
        restic.path(),
        EXPECTED_SHA256,
        &["version".to_owned()],
        &[],
        &crate::paths::cache_dir(),
    )?;
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let output = command.output().context("cannot run restic")?;
    drop(file);
    Ok(output)
}

/// Opens the binary, checks the hash through the handle and starts the child from it (threat T20).
fn spawn_verified(
    path: &Path,
    expected_hex: &str,
    line: &CommandLine,
    cache: &Path,
) -> Result<Child> {
    let (file, mut command) = verified_command(path, expected_hex, line.args(), line.env(), cache)?;
    let child = command.spawn().context("cannot start restic")?;
    // The handle stays open until the child has started, so the checked inode is the one that runs.
    drop(file);
    Ok(child)
}

/// Starts the verified restic child of one job with its standard streams detached (FR-AG-06,
/// threat T9, threat T20).
///
/// A restic message can carry the repository location, so the agent reads the exit code only and
/// never the output. The hash check and the start share one opening of the binary, as in
/// [`spawn_verified`]. The child runs in its own process group, so the agent stops its whole tree
/// (FR-AG-10).
///
/// # Errors
///
/// Returns an error when the hash differs or the process cannot start.
pub(crate) fn spawn_job_restic(
    path: &Path,
    expected_hex: &str,
    line: &CommandLine,
    cache: &Path,
) -> Result<JobChild> {
    let (file, mut command) = verified_command(path, expected_hex, line.args(), line.env(), cache)?;
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let child = command.spawn().context("cannot start restic")?;
    drop(file);
    JobChild::new(child)
}

/// The restic child of one job and the handle that stops its whole tree (FR-AG-10).
pub(crate) struct JobChild {
    child: Child,
    /// Holds the tree on Windows, so closing it kills the tree (FR-AG-10).
    #[cfg(windows)]
    job: Option<windows_job::JobObject>,
}

impl JobChild {
    /// Wraps a spawned child and, on Windows, holds its tree in a job object (FR-AG-10).
    fn new(child: Child) -> Result<Self> {
        #[cfg(windows)]
        let job = windows_job::create(&child);
        Ok(Self {
            child,
            #[cfg(windows)]
            job,
        })
    }

    /// Reports the exit of the child, or `None` while it runs.
    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Sends the graceful stop signal to the whole process group (FR-AG-10).
    ///
    /// It is `SIGINT` on Unix and `CTRL_BREAK` on Windows. A child that ignores it keeps running
    /// until [`Self::kill_tree`].
    #[allow(unsafe_code)]
    pub(crate) fn interrupt(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            let group = i32::try_from(self.child.id()).unwrap_or(i32::MAX);
            // SAFETY: a negative pid sends the signal to the process group that the child leads.
            if unsafe { libc::kill(-group, libc::SIGINT) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
            // SAFETY: the child runs in the new process group whose id is its pid.
            if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.child.id()) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
    }

    /// Stops the whole process tree of the child, even after it ignored the graceful signal
    /// (FR-AG-10).
    #[allow(unsafe_code)]
    pub(crate) fn kill_tree(&mut self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            let group = i32::try_from(self.child.id()).unwrap_or(i32::MAX);
            // SAFETY: a negative pid sends the signal to the process group that the child leads.
            if unsafe { libc::kill(-group, libc::SIGKILL) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            // The job object kills the whole tree; a missing job object falls back to the child.
            if let Some(job) = &self.job
                && job.terminate()
            {
                return Ok(());
            }
            self.child.kill()
        }
    }
}

/// The Windows job object that holds the restic tree of one job (FR-AG-10).
///
/// The object carries `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so the whole tree dies when the agent
/// closes the handle, even after an unexpected stop of the agent (AC-AG-07).
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_job {
    use std::os::windows::io::AsRawHandle as _;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };

    /// Owns the job object and closes it on drop, which kills the tree (FR-AG-10).
    pub(super) struct JobObject(HANDLE);

    // A Windows handle is a plain value that any thread may use, so the job travels with the
    // `JobChild` across the runtime threads.
    unsafe impl Send for JobObject {}
    unsafe impl Sync for JobObject {}

    impl Drop for JobObject {
        fn drop(&mut self) {
            // SAFETY: the handle comes from `CreateJobObjectW` and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl JobObject {
        /// Kills every process of the tree (FR-AG-10). False when the call fails.
        pub(super) fn terminate(&self) -> bool {
            // SAFETY: the handle is a valid job object and 1 is the exit code.
            unsafe { TerminateJobObject(self.0, 1) != 0 }
        }
    }

    /// Creates a job object for `child` with `KILL_ON_JOB_CLOSE` (FR-AG-10).
    ///
    /// A failure gives `None`, so the stop falls back to the direct child only.
    pub(super) fn create(child: &Child) -> Option<JobObject> {
        // SAFETY: the two pointers are null, so Windows uses the default attributes and no name.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let job = JobObject(handle);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let length =
            u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).ok()?;
        // SAFETY: the job object is valid and the information pointer is a local of the right size.
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                length,
            )
        };
        if ok == 0 {
            return None;
        }
        // The process handle from `Child` is valid while the child lives.
        let process = child.as_raw_handle() as HANDLE;
        // SAFETY: both handles are valid; the child joins the job and cannot leave it.
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            return None;
        }
        Some(job)
    }
}

/// Opens the binary, checks the hash through the handle and builds the child command (threat T20).
///
/// The returned file holds the checked descriptor; the caller keeps it open until the child starts.
/// The cache directory is created after the check, so an unverified binary never creates one.
fn verified_command(
    path: &Path,
    expected_hex: &str,
    args: &[String],
    env: &[(String, String)],
    cache: &Path,
) -> Result<(std::fs::File, Command)> {
    let file = open_for_exec(path)?;
    let digest = digest_of(&file)?;
    crate::restic::verify_digest(&digest, expected_hex)
        .with_context(|| format!("the bundled restic at {}", path.display()))?;
    crate::paths::create_protected_dir(cache)?;
    let command = restic_command_with(&pinned_program(&file, path), args, env, cache);
    Ok((file, command))
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

/// Opens the binary for the hash check. macOS and other systems have no handle pin.
#[cfg(not(any(target_os = "linux", windows)))]
fn open_for_exec(path: &Path) -> Result<std::fs::File> {
    std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))
}

/// The program that runs the checked file (threat T20).
///
/// The descriptor keeps its close-on-exec flag: the kernel resolves `/proc/self/fd/<fd>` to the
/// checked file before it closes the descriptor at the exec, so the child runs the checked inode
/// and holds no extra descriptor.
#[cfg(target_os = "linux")]
fn pinned_program(file: &std::fs::File, _path: &Path) -> PathBuf {
    use std::os::fd::AsRawFd as _;
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

/// The program that runs the checked file on a system without a descriptor pin (threat T20).
#[cfg(not(target_os = "linux"))]
fn pinned_program(_file: &std::fs::File, path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
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
        let mut child = spawn_verified(&program, &expected, &line, &cache).expect("cannot start");
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
            format!("{error:#}").contains("does not match the pinned SHA-256"),
            "{error:#}"
        );
        let _ = std::fs::remove_file(&path);
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

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(text, "{byte:02x}");
        }
        text
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
        let script =
            "trap '' INT; ( trap '' INT; echo $$ > \"$1\"; while :; do sleep 1; done ) & wait";

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
}
