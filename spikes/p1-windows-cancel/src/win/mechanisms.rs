//! The mechanisms A, B, C1, C2 and D.
//!
//! Each mechanism starts one `restic backup`, waits for its lock, sends the
//! stop in its own way, and records the result.

use std::fs::File;
use std::io;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;

use super::Config;
use super::api::{self, JobObject, RunningChild};
use super::report::{self, ApiCall, LockProbe, MechanismResult, Ready};

/// An action before the child starts.
pub enum PreSpawn {
    /// Allocate a console for the spike process.
    AllocConsole,
}

/// How the spike creates the restic child.
pub enum Spawn {
    /// A `std::process::Command` child with these creation flags.
    Std(u32),
    /// A direct `CreateProcessW` child with `CREATE_NEW_CONSOLE` and a hidden
    /// window.
    RawHiddenConsole,
}

/// How the spike stops the restic child.
pub enum Stop {
    /// Send `CTRL_BREAK_EVENT` directly to the child process group.
    CtrlBreak,
    /// Start `p1-cancel break-helper` and let it send `CTRL_BREAK_EVENT`.
    BreakHelper,
    /// Terminate the job object of the child. This is the hard kill baseline.
    TerminateJob,
}

/// One mechanism under test.
pub struct Spec {
    pub id: &'static str,
    pub description: &'static str,
    pub spawn: Spawn,
    pub pre_spawn: Option<PreSpawn>,
    pub stop: Stop,
}

/// Run one mechanism and return its result.
pub fn run(spec: &Spec, cfg: &Config) -> Result<MechanismResult, String> {
    let mut api_calls: Vec<ApiCall> = Vec::new();
    let mut notes = String::new();

    if let Some(PreSpawn::AllocConsole) = spec.pre_spawn {
        record(&mut api_calls, "AllocConsole()", api::alloc_console());
        record(
            &mut api_calls,
            "GetConsoleWindow() + ShowWindow(SW_HIDE)",
            api::hide_console_window(),
        );
    }

    // A new repository for each mechanism. A previous snapshot cannot make the
    // next backup finish early.
    let repo = fresh_repo(cfg, spec.id)?;

    let temp = std::env::temp_dir();
    let stdout_path = temp.join(format!("p1-{}-{}-stdout.log", cfg.mode.as_str(), spec.id));
    let stderr_path = temp.join(format!("p1-{}-{}-stderr.log", cfg.mode.as_str(), spec.id));
    let stdout_file = File::create(&stdout_path)
        .map_err(|error| format!("create {}: {error}", stdout_path.display()))?;
    let stderr_file = File::create(&stderr_path)
        .map_err(|error| format!("create {}: {error}", stderr_path.display()))?;

    let mut child = match spec.spawn {
        Spawn::Std(flags) => spawn_std(cfg, &repo, flags, stdout_file, stderr_file)?,
        Spawn::RawHiddenConsole => api::spawn_raw(
            &cfg.restic,
            &backup_args(cfg, &repo),
            CREATE_NEW_CONSOLE,
            &stdout_file,
            &stderr_file,
        )
        .map_err(|error| format!("CreateProcessW with a hidden console: {error}"))?,
    };
    let pid = child.pid();

    let job = JobObject::create().map_err(|error| format!("CreateJobObject: {error}"))?;
    job.assign(child.handle())
        .map_err(|error| format!("AssignProcessToJobObject: {error}"))?;

    let ready = wait_ready(&stdout_path, Duration::from_secs(5));
    let lock_before = wait_for_lock(cfg, &repo, Duration::from_secs(10));
    let locks_before_stop = lock_before.count;
    if locks_before_stop <= 0 {
        notes.push_str("the repository held no lock before the stop; the run is not valid. ");
    }

    // The child may have finished before the stop. Record this fact and send no
    // stop in that case.
    let exited_before_stop = matches!(child.try_wait(), Ok(Some(_)));

    let started = Instant::now();
    if exited_before_stop {
        notes.push_str("the child exited before the stop; the spike sent no stop. ");
    } else {
        match spec.stop {
            Stop::CtrlBreak => {
                let name = format!("GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, {pid})");
                record(&mut api_calls, &name, api::generate_break(pid));
            }
            Stop::BreakHelper => {
                let call = stop_with_helper(pid)?;
                api_calls.push(call);
            }
            Stop::TerminateJob => {
                record(
                    &mut api_calls,
                    "TerminateJobObject(child)",
                    job.terminate(1),
                );
            }
        }
    }

    let exit_code = wait_child(&mut child, Duration::from_secs(30));
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let exited_within_30s = exit_code.is_some();
    if !exited_within_30s {
        let _ = job.terminate(1);
    }

    let lock_after = report::count_locks(&cfg.restic, &repo);
    let locks_after_stop = lock_after.count;
    let saw_summary = file_contains(&stdout_path, "\"message_type\":\"summary\"");
    let restic_stderr_tail = read_tail(&stderr_path, 20);
    let interrupt_message = stderr_has_interrupt(&stderr_path);
    let _ = report::unlock(&cfg.restic, &repo);

    // Record which signal restic used, when it was graceful.
    let graceful_signal = match (exit_code, interrupt_message) {
        (Some(130), _) => Some("exit_code_130".to_string()),
        (Some(1), true) => Some("exit_code_1_interrupt_message".to_string()),
        _ => None,
    };

    let mut result = MechanismResult {
        mechanism: spec.id.to_string(),
        mode: cfg.mode.as_str().to_string(),
        description: spec.description.to_string(),
        repository: repo.to_string_lossy().into_owned(),
        api_calls,
        ready,
        exited_before_stop,
        saw_summary,
        interrupt_message,
        graceful_signal,
        locks_before_stop,
        locks_after_stop,
        locks_after_stop_stderr_tail: lock_after.stderr_tail,
        exited_within_30s,
        exit_code,
        elapsed_ms,
        restic_stderr_tail,
        verdict: String::new(),
        notes,
    };
    let verdict = result.compute_verdict().to_string();
    result.verdict = verdict;
    Ok(result)
}

/// Create the new repository of one mechanism.
fn fresh_repo(cfg: &Config, id: &str) -> Result<PathBuf, String> {
    let repo = cfg.repo.join(id);
    if repo.exists() {
        std::fs::remove_dir_all(&repo)
            .map_err(|error| format!("remove {}: {error}", repo.display()))?;
    }
    std::fs::create_dir_all(&repo)
        .map_err(|error| format!("create {}: {error}", repo.display()))?;
    report::init(&cfg.restic, &repo)?;
    Ok(repo)
}

fn spawn_std(
    cfg: &Config,
    repo: &Path,
    flags: u32,
    stdout: File,
    stderr: File,
) -> Result<RunningChild, String> {
    let mut command = Command::new(&cfg.restic);
    command
        .arg("-r")
        .arg(repo)
        .arg("backup")
        .arg("--json")
        .arg(&cfg.source);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    command.creation_flags(flags);
    command
        .spawn()
        .map(RunningChild::from_std)
        .map_err(|error| format!("spawn restic: {error}"))
}

fn backup_args(cfg: &Config, repo: &Path) -> Vec<String> {
    vec![
        "-r".to_string(),
        repo.to_string_lossy().into_owned(),
        "backup".to_string(),
        "--json".to_string(),
        cfg.source.to_string_lossy().into_owned(),
    ]
}

/// Start the `break-helper` child and wait for it.
///
/// The stderr of the helper is read on a separate thread, so that a stuck
/// helper cannot block the harness. After a timeout the spike stops the helper.
fn stop_with_helper(pid: u32) -> Result<ApiCall, String> {
    let executable = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    let mut command = Command::new(executable);
    command
        .arg("break-helper")
        .arg("--pid")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn break-helper: {error}"))?;

    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    });

    let mut running = RunningChild::from_std(child);
    if let Ok(job) = JobObject::create() {
        let _ = job.assign(running.handle());
    }

    let helper_exit = wait_child(&mut running, Duration::from_secs(10));
    if helper_exit.is_none() {
        let _ = running.kill();
    }
    let helper_stderr = reader.join().unwrap_or_default();

    Ok(ApiCall {
        call: format!("break-helper --pid {pid}"),
        result: format!(
            "helper exit code {helper_exit:?}, stderr: {}",
            helper_stderr.trim()
        ),
        last_error: None,
    })
}

/// Wait for the first JSON `status` line or for the time limit.
fn wait_ready(stdout_path: &Path, limit: Duration) -> Ready {
    let start = Instant::now();
    let mut saw_status_line = false;
    while start.elapsed() < limit {
        if file_contains(stdout_path, "\"message_type\":\"status\"") {
            saw_status_line = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ready {
        saw_status_line,
        waited_ms: start.elapsed().as_millis() as u64,
    }
}

/// Wait until the repository has a lock, or for the time limit.
fn wait_for_lock(cfg: &Config, repo: &Path, limit: Duration) -> LockProbe {
    let start = Instant::now();
    loop {
        let probe = report::count_locks(&cfg.restic, repo);
        if probe.count != 0 || start.elapsed() >= limit {
            return probe;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Wait for the child to exit, at most for the time limit.
fn wait_child(child: &mut RunningChild, limit: Duration) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Ok(Some(code)) = child.try_wait() {
            return Some(code);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn record(calls: &mut Vec<ApiCall>, name: &str, result: io::Result<()>) {
    match result {
        Ok(()) => calls.push(ApiCall {
            call: name.to_string(),
            result: "ok".to_string(),
            last_error: None,
        }),
        Err(error) => calls.push(ApiCall {
            call: name.to_string(),
            result: format!("failed: {error}"),
            last_error: error.raw_os_error().map(|code| code as u32),
        }),
    }
}

fn file_contains(path: &Path, needle: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.contains(needle))
        .unwrap_or(false)
}

/// True when the file holds an interrupt message. restic writes
/// "signal interrupt received, cleaning up" on an interrupt.
fn stderr_has_interrupt(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.to_lowercase().contains("interrupt"))
        .unwrap_or(false)
}

fn read_tail(path: &Path, count: usize) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines
        .iter()
        .rev()
        .take(count)
        .rev()
        .map(|line| line.to_string())
        .collect()
}
