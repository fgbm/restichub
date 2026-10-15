//! Hardening of the restic child process (FR-AG-06, FR-AG-11, ADR-0021, ADR-0031).
//!
//! The agent starts the installed restic with a cleared environment, the variables of the step
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
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};

use anyhow::{Context, Result};
use restichub_compiler::arg::CommandLine;
use restichub_proto::agent::v1::ProcessPriority;

use crate::restic::ResticBinary;

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
/// pid. On Linux the kernel also kills the child when the agent dies. On Windows the child becomes a
/// new process group and the agent sends it `CTRL_BREAK`.
fn apply_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(target_os = "linux")]
    die_with_parent(command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(windows_job::process_flags());
    }
}

/// Makes the kernel kill the child with `SIGKILL` when the agent dies (AC-AG-07).
///
/// The kernel ties the signal to the thread that started the child, so the agent starts a restic
/// child only from a thread that lives as long as the runtime.
#[cfg(target_os = "linux")]
#[allow(unsafe_code, clippy::cast_sign_loss)]
fn die_with_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: `getpid` has no preconditions.
    let parent = unsafe { libc::getpid() };
    // SAFETY: the closure runs between fork and exec and calls only async-signal-safe functions.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0,
                0,
                0,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            // The agent may have died before the call above, and then nothing sends the signal.
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

/// Allocates a hidden console for a process that has none, so `CTRL_BREAK` reaches a restic child
/// (FR-AG-10).
///
/// A Windows service has no console, and `GenerateConsoleCtrlEvent` fails with an invalid handle
/// without one (spike P1). The restic children inherit this console. The window stays hidden.
///
/// # Errors
///
/// Returns an error when Windows refuses the console.
#[cfg(windows)]
pub fn allocate_hidden_console() -> Result<()> {
    windows_job::allocate_hidden_console().context("cannot allocate the console")
}

/// Re-checks the recorded hash and starts the child process (FR-AG-11, threat T20).
///
/// # Errors
///
/// Returns an error when the hash differs or the process cannot start.
pub fn spawn_restic(restic: &ResticBinary, line: &CommandLine) -> Result<Child> {
    spawn_verified(
        restic.path(),
        restic.expected_sha256(),
        line,
        &crate::paths::cache_dir(),
    )
}

/// Runs the recorded restic with `version` and collects its output (FR-AG-11, threat T20).
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
        restic.expected_sha256(),
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

/// Starts the verified restic child of one job with piped output (FR-AG-06, FR-AG-08, threat T9,
/// threat T20).
///
/// A restic message can carry the repository location, so the caller scrubs every line before it
/// sends or stores it ([`crate::scrub`]). The hash check and the start share one opening of the
/// binary, as in [`spawn_verified`]. The child runs in its own process group, so the agent stops
/// its whole tree (FR-AG-10). It has no standard input.
///
/// # Errors
///
/// Returns an error when the hash differs or the process cannot start.
pub(crate) fn spawn_job_restic(
    path: &Path,
    expected_hex: &str,
    line: &CommandLine,
    cache: &Path,
    priority: ProcessPriority,
) -> Result<JobChild> {
    let (file, mut command) = verified_command(path, expected_hex, line.args(), line.env(), cache)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_priority(&mut command, priority);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::System::Threading::{
            BELOW_NORMAL_PRIORITY_CLASS, CREATE_SUSPENDED,
        };
        // The child starts suspended and joins the job object before it runs, so no grandchild
        // escapes the job (FR-AG-10).
        let class = if priority == ProcessPriority::Low {
            BELOW_NORMAL_PRIORITY_CLASS
        } else {
            0
        };
        command.creation_flags(windows_job::process_flags() | CREATE_SUSPENDED | class);
    }
    let child = command.spawn().context("cannot start restic")?;
    drop(file);
    JobChild::new(child)
}

/// Starts one hook child with piped output (FR-HK-03, FR-HK-05, ADR-0004).
///
/// The program, the arguments and the environment come from [`crate::hooks::Invocation`], which
/// the allow-list built. The child has no shell and no standard input. Its environment is cleared:
/// it gets a fixed search path, the variables of the allow-list and the variables that the schema
/// names. Nothing of the agent environment reaches it. A server script is the exception: the host
/// trusts it, so it keeps the environment of the agent without the restic and ResticHub variables
/// (ADR-0037). It runs in its own process group, so the
/// agent stops its whole tree (FR-AG-10). The invocation can replace `PATH`, and on Windows the
/// child gets a protected directory as `TEMP` and `TMP`.
///
/// # Errors
///
/// Returns an error when the operating system refuses to start the process.
pub(crate) fn spawn_job_hook(
    invocation: &crate::hooks::Invocation,
    scratch: &Path,
) -> Result<JobChild> {
    let mut command = Command::new(&invocation.program);
    if invocation.inherit_environment {
        for (name, _) in std::env::vars_os() {
            let upper = name.to_string_lossy().to_ascii_uppercase();
            if upper.starts_with("RESTIC") {
                command.env_remove(&name);
            }
        }
    } else {
        command.env_clear();
    }
    #[cfg(windows)]
    {
        if !invocation.inherit_environment {
            let system_root = std::env::var_os("SystemRoot")
                .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows"));
            let mut search_path = PathBuf::from(&system_root);
            search_path.push("System32");
            command.env("PATH", search_path);
            command.env("SystemRoot", system_root);
        }
        // The service `TEMP` can be a directory that a user can write.
        let temp = if invocation.inherit_environment {
            &invocation.working_dir
        } else {
            scratch
        };
        command.env("TEMP", temp);
        command.env("TMP", temp);
    }
    #[cfg(not(windows))]
    {
        let _ = scratch;
        if !invocation.inherit_environment || std::env::var_os("PATH").is_none() {
            command.env(
                "PATH",
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            );
        }
    }
    if let Some(search_path) = &invocation.search_path {
        command.env("PATH", search_path);
    }
    for (name, value) in &invocation.env {
        command.env(name, value.as_str());
    }
    command
        .args(&invocation.args)
        .current_dir(&invocation.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_process_group(&mut command);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
        // The child joins the job object before it runs, so no grandchild escapes (FR-AG-10).
        command.creation_flags(windows_job::process_flags() | CREATE_SUSPENDED);
    }
    let child = command.spawn().context("cannot start the hook")?;
    JobChild::new(child)
}

/// Lowers the process and I/O priority of the child when the step asks for it (FR-AG-15).
///
/// On Linux the child gets a nice value 10 above the agent, capped at 19, and the best-effort I/O
/// class at level 7. A failure of either
/// call is ignored: the priority is a courtesy to the host, not a safety rule. On Windows the
/// creation flags carry the priority class.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn apply_priority(command: &mut Command, priority: ProcessPriority) {
    use std::os::unix::process::CommandExt as _;
    if priority != ProcessPriority::Low {
        return;
    }
    // SAFETY: the closure runs between fork and exec and calls only async-signal-safe functions.
    unsafe {
        command.pre_exec(|| {
            let current = libc::getpriority(libc::PRIO_PROCESS, 0);
            libc::setpriority(libc::PRIO_PROCESS, 0, lowered_nice(current));
            libc::syscall(
                libc::SYS_ioprio_set,
                IOPRIO_WHO_PROCESS,
                0,
                (IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT) | LOW_IO_LEVEL,
            );
            Ok(())
        });
    }
}

/// How far above the nice value of the agent a low-priority child runs (FR-AG-15).
#[cfg(target_os = "linux")]
const LOW_NICE_STEP: libc::c_int = 10;

/// Nice 19 is the lowest priority that Linux allows.
#[cfg(target_os = "linux")]
const MAX_NICE: libc::c_int = 19;

/// The nice value of a low-priority child: a step above the agent, so an agent that already runs
/// at a low priority does not get a child that runs before it (FR-AG-15).
#[cfg(target_os = "linux")]
fn lowered_nice(current: libc::c_int) -> libc::c_int {
    current.saturating_add(LOW_NICE_STEP).min(MAX_NICE)
}

/// The best-effort I/O level of a low-priority child; level 7 is the lowest (FR-AG-15).
#[cfg(target_os = "linux")]
const LOW_IO_LEVEL: libc::c_long = 7;

/// The `ioprio_set` target selector for one process.
#[cfg(target_os = "linux")]
const IOPRIO_WHO_PROCESS: libc::c_long = 1;

/// Best-effort I/O scheduling class of `ioprio_set`.
#[cfg(target_os = "linux")]
const IOPRIO_CLASS_BE: libc::c_long = 2;

/// The bit position of the class in an I/O priority value.
#[cfg(target_os = "linux")]
const IOPRIO_CLASS_SHIFT: libc::c_long = 13;

/// A system without a priority call runs the child at the default priority.
#[cfg(not(target_os = "linux"))]
fn apply_priority(_command: &mut Command, _priority: ProcessPriority) {}

/// The restic child of one job and the handle that stops its whole tree (FR-AG-10).
pub(crate) struct JobChild {
    child: Child,
    /// Holds the tree on Windows, so closing it kills the tree (FR-AG-10).
    #[cfg(windows)]
    job: Option<windows_job::JobObject>,
}

impl JobChild {
    /// Wraps a spawned child and, on Windows, holds its tree in a job object (FR-AG-10).
    ///
    /// On Windows the child must start suspended: it joins the job object and then resumes. A child
    /// that cannot join the job is killed, because the agent could not stop its tree.
    fn new(#[cfg_attr(not(windows), allow(unused_mut))] mut child: Child) -> Result<Self> {
        #[cfg(windows)]
        let job = match windows_job::contain(&child) {
            Ok(job) => Some(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("cannot hold the restic tree in a job object");
            }
        };
        Ok(Self {
            child,
            #[cfg(windows)]
            job,
        })
    }

    /// Takes the output pipes of the child. A second call gives `None` for both.
    pub(crate) fn take_output(&mut self) -> (Option<ChildStdout>, Option<ChildStderr>) {
        (self.child.stdout.take(), self.child.stderr.take())
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

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{
        AllocConsole, GetConsoleProcessList, GetConsoleWindow,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};

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

    /// The creation flags of a restic child: a new process group for `CTRL_BREAK` (FR-AG-10).
    ///
    /// A child of a process without a console gets a visible console window of its own, which
    /// `CREATE_NO_WINDOW` prevents. A child of a process with a console must inherit it, because
    /// `CTRL_BREAK` reaches only a child on the console of the sender.
    pub(super) fn process_flags() -> u32 {
        if has_console() {
            CREATE_NEW_PROCESS_GROUP
        } else {
            CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        }
    }

    /// True when the process is attached to a console.
    fn has_console() -> bool {
        let mut process = 0_u32;
        // SAFETY: the buffer holds one element, as the size argument says.
        unsafe { GetConsoleProcessList(&mut process, 1) != 0 }
    }

    /// Allocates a console and hides its window, unless the process already has a console.
    pub(super) fn allocate_hidden_console() -> std::io::Result<()> {
        if has_console() {
            return Ok(());
        }
        // SAFETY: the call takes no pointer.
        if unsafe { AllocConsole() } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the window handle comes from the call before and `ShowWindow` accepts any value.
        unsafe {
            let window = GetConsoleWindow();
            if !window.is_null() {
                ShowWindow(window, SW_HIDE);
            }
        }
        Ok(())
    }

    /// Creates a job object with `KILL_ON_JOB_CLOSE`, puts the suspended `child` in it and resumes
    /// the child (FR-AG-10).
    ///
    /// The child joins the job before its first instruction, so every grandchild is in the job too.
    pub(super) fn contain(child: &Child) -> std::io::Result<JobObject> {
        // SAFETY: the two pointers are null, so Windows uses the default attributes and no name.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = JobObject(handle);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let length = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(|_| std::io::Error::other("the job limit structure is too large"))?;
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
            return Err(std::io::Error::last_os_error());
        }
        // The process handle from `Child` is valid while the child lives.
        let process = child.as_raw_handle() as HANDLE;
        // SAFETY: both handles are valid; the child joins the job and cannot leave it.
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        resume_threads(child.id())?;
        Ok(job)
    }

    /// Resumes the threads of a process that started suspended.
    ///
    /// `std` gives no handle to the main thread, so the function finds the threads of the process
    /// with a snapshot.
    fn resume_threads(process_id: u32) -> std::io::Result<()> {
        // SAFETY: the call takes no pointer.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry = THREADENTRY32 {
            dwSize: u32::try_from(std::mem::size_of::<THREADENTRY32>()).unwrap_or(0),
            ..THREADENTRY32::default()
        };
        let mut resumed = false;
        // SAFETY: the snapshot is valid and the entry is a local with its size set.
        let mut more = unsafe { Thread32First(snapshot, &mut entry) };
        while more != 0 {
            if entry.th32OwnerProcessID == process_id {
                // SAFETY: the thread identifier comes from the snapshot.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if !thread.is_null() {
                    // SAFETY: the thread handle is valid and closed once.
                    unsafe {
                        resumed |= ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                }
            }
            // SAFETY: the snapshot is valid and the entry is a local with its size set.
            more = unsafe { Thread32Next(snapshot, &mut entry) };
        }
        // SAFETY: the snapshot handle is closed once.
        unsafe { CloseHandle(snapshot) };
        if resumed {
            Ok(())
        } else {
            Err(std::io::Error::other("cannot resume the restic child"))
        }
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
        .with_context(|| format!("the restic at {}", path.display()))?;
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
            format!("{stale:#}").contains("restart the service"),
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

    /// FR-AG-15: the low priority is relative to the agent and never passes the highest nice value.
    #[cfg(target_os = "linux")]
    #[test]
    fn fr_ag_15_the_low_priority_is_relative_to_the_agent() {
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
}
