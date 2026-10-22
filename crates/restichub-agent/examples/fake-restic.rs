//! A scripted restic double for the agent job tests.
//!
//! The agent runs a step with the repository location in `RESTIC_REPOSITORY`. The double reads a
//! sibling file `<location>.restichub-fake` that holds two exit codes: the `test_access` code and
//! the `init` code. It appends its own arguments to `<location>.restichub-argv`, so a test can prove
//! that no repository secret reaches an argument (AC-AG-04). It appends the names of the rest-server
//! credential variables that it gets to `<location>.restichub-env`, never their values. It exits
//! with the code of the step.
//!
//! Two optional sibling files change the timing:
//!
//! - `<location>.restichub-sleep` holds a duration in milliseconds; the double waits it before it
//!   exits, so a test can drop the stream or cancel the job while the step runs.
//! - `<location>.restichub-ignore` makes the double ignore `SIGINT` on Linux and `CTRL_BREAK` on
//!   Windows, so a test covers the forced kill of the process tree (FR-AG-10, AC-AG-07). Without
//!   it the double exits with code 130 on `CTRL_BREAK`.
//!
//! - `<location>.restichub-signal` receives one line with the graceful stop signal that the double
//!   got, so a test proves that the agent stopped restic instead of killing it (FR-AG-10,
//!   AC-AG-06).
//!
//! - `<location>.restichub-kill` makes the double end itself with `SIGKILL` after the sleep, as an
//!   outside kill does (AC-SC-06, Linux only).
//!
//! Further optional sibling files script the output of one restic command; `<command>` is the first
//! argument, such as `backup`, `check` or `snapshots`:
//!
//! - `<location>.restichub-out-<command>` holds the text for the standard output and
//!   `<location>.restichub-err-<command>` the text for the standard error. A line `#sleep <ms>`
//!   waits that long. `{PASSWORD}` and `{REPOSITORY}` stand for the environment values, so a test
//!   proves that the agent scrubs a secret that restic echoes.
//! - `<location>.restichub-code-<command>` holds the exit code of that command and wins over the
//!   codes of `<location>.restichub-fake`.
//! - `<location>.restichub-ls-count` holds a number. The `ls` command then prints the `snapshot`
//!   message, the listed directory and that many `node` messages, generated one by one. It stops
//!   at the first write error, as restic does when the agent stops reading. A test streams a
//!   million entries without a file of that size (AC-RS-02).
//! - `<location>.restichub-state` receives the nice value and the I/O priority of the process, so
//!   a test proves the priority of a step (FR-AG-15, Linux only).
//!
//! The binary is a plain example, so both the Linux and the Windows runner build it and the agent's
//! hash check accepts it, as in production (ADR-0031).

use std::io::Write as _;
use std::time::Duration;

fn main() {
    let repository = std::env::var("RESTIC_REPOSITORY").unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().cloned().unwrap_or_default();
    record(&format!("{repository}.restichub-argv"), &args);
    record_credential_names(&format!("{repository}.restichub-env"));
    record_priority(&format!("{repository}.restichub-state"));
    ignore_interrupt(
        &format!("{repository}.restichub-ignore"),
        &format!("{repository}.restichub-signal"),
    );
    if command == "ls" {
        emit_listing(&format!("{repository}.restichub-ls-count"), &args);
    }
    emit(
        &format!("{repository}.restichub-out-{command}"),
        &repository,
        false,
    );
    emit(
        &format!("{repository}.restichub-err-{command}"),
        &repository,
        true,
    );
    sleep_for(&format!("{repository}.restichub-sleep"));
    kill_self(&format!("{repository}.restichub-kill"));
    let codes = std::fs::read_to_string(format!("{repository}.restichub-fake")).unwrap_or_default();
    let mut codes = codes.split_whitespace();
    let test_access = codes
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let init = codes
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let scripted = std::fs::read_to_string(format!("{repository}.restichub-code-{command}"))
        .ok()
        .and_then(|value| value.trim().parse().ok());
    let code = scripted.unwrap_or(if command == "init" { init } else { test_access });
    std::process::exit(code);
}

/// Ends the process with `SIGKILL` when the control file exists.
fn kill_self(path: &str) {
    if cfg!(unix) && std::path::Path::new(path).exists() {
        let _ = std::process::Command::new("kill")
            .args(["-9", &std::process::id().to_string()])
            .status();
        std::thread::sleep(Duration::from_secs(30));
    }
}

/// Prints a generated `ls` listing of the directory in the last argument.
fn emit_listing(path: &str, args: &[String]) {
    let Some(count) = std::fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    else {
        return;
    };
    let directory = args
        .last()
        .map_or("/", String::as_str)
        .trim_end_matches('/');
    let mut out = std::io::BufWriter::with_capacity(64 * 1024, std::io::stdout().lock());
    let snapshot = r#"{"time":"2026-01-01T00:00:00Z","tree":"5c1d","paths":["/data"],"hostname":"h","username":"u","id":"b2f8a1c94e7d60351a8f2b4c6d9e0f3a7c5b1d2e4f6089a3b7c1d5e9f2a406b8","short_id":"b2f8a1c9","message_type":"snapshot","struct_type":"snapshot"}"#;
    if writeln!(out, "{snapshot}").is_err() {
        return;
    }
    if !directory.is_empty()
        && writeln!(
            out,
            r#"{{"name":"d","type":"dir","path":"{directory}","uid":1,"gid":1,"mode":2147484157,"permissions":"drwxr-xr-x","message_type":"node","struct_type":"node"}}"#
        )
        .is_err()
    {
        return;
    }
    for index in 0..count {
        let line = format!(
            r#"{{"name":"file-{index:07}","type":"file","path":"{directory}/file-{index:07}","uid":1,"gid":1,"size":{index},"mode":420,"permissions":"-rw-r--r--","mtime":"2026-01-01T00:00:00Z","message_type":"node","struct_type":"node"}}"#
        );
        if writeln!(out, "{line}").is_err() {
            return;
        }
    }
    let _ = out.flush();
}

/// Prints the scripted text of the file to the standard output or the standard error.
fn emit(path: &str, repository: &str, to_error: bool) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let password = std::env::var("RESTIC_PASSWORD").unwrap_or_default();
    for line in text.lines() {
        if let Some(millis) = line.strip_prefix("#sleep ") {
            if let Ok(millis) = millis.trim().parse::<u64>() {
                std::thread::sleep(Duration::from_millis(millis));
            }
            continue;
        }
        let line = line
            .replace("{PASSWORD}", &password)
            .replace("{REPOSITORY}", repository);
        if to_error {
            let mut stream = std::io::stderr().lock();
            let _ = writeln!(stream, "{line}");
            let _ = stream.flush();
        } else {
            let mut stream = std::io::stdout().lock();
            let _ = writeln!(stream, "{line}");
            let _ = stream.flush();
        }
    }
}

/// Appends the nice value and the I/O priority of this process.
#[allow(unsafe_code)]
fn record_priority(path: &str) {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: both calls read the priority of this process.
        let (nice, ioprio) = unsafe {
            (
                libc::getpriority(libc::PRIO_PROCESS, 0),
                libc::syscall(libc::SYS_ioprio_get, 1, 0),
            )
        };
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        else {
            return;
        };
        let _ = writeln!(
            file,
            "nice={nice} io_class={} io_level={}",
            ioprio >> 13,
            ioprio & 7
        );
    }
    #[cfg(not(target_os = "linux"))]
    let _ = path;
}

/// The file that receives the graceful stop signal. A handler on another thread writes it, so the
/// path is a process-wide value.
static SIGNAL_FILE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Appends the name of the graceful stop signal that this process got.
fn record_signal(name: &str) {
    let Some(path) = SIGNAL_FILE.get() else {
        return;
    };
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    if let Ok(mut file) = opened {
        let _ = writeln!(file, "{name}");
    }
}

/// Ignores the interrupt when the marker file asks for it, so only the forced kill stops the double.
///
/// On Windows the double handles `CTRL_BREAK`: it exits with code 130 as restic does, or ignores the
/// event when the marker asks for it. The same handler records the signal in `signal`.
#[allow(unsafe_code)]
fn ignore_interrupt(path: &str, signal: &str) {
    let ignore = std::fs::read_to_string(path).is_ok_and(|value| !value.trim().is_empty());
    let _ = SIGNAL_FILE.set(signal.to_owned());
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, SetConsoleCtrlHandler};

        static IGNORE: AtomicBool = AtomicBool::new(false);

        unsafe extern "system" fn handler(kind: u32) -> i32 {
            if kind != CTRL_BREAK_EVENT {
                return 0;
            }
            if IGNORE.load(Ordering::SeqCst) {
                return 1;
            }
            record_signal("ctrl_break");
            std::process::exit(130)
        }

        IGNORE.store(ignore, Ordering::SeqCst);
        // SAFETY: the handler is a plain function that lives for the whole process.
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }
    #[cfg(target_os = "linux")]
    if ignore {
        // SAFETY: `signal` installs the standard ignore handler for SIGINT.
        unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    }
    #[cfg(target_os = "linux")]
    if !ignore {
        // SAFETY: `signal` installs a plain function that lives for the whole process.
        unsafe {
            libc::signal(
                libc::SIGINT,
                record_sigint as *const () as libc::sighandler_t,
            )
        };
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = ignore;
}

/// Records the graceful stop and exits as restic does after `SIGINT`.
#[cfg(target_os = "linux")]
extern "C" fn record_sigint(_signal: libc::c_int) {
    record_signal("sigint");
    std::process::exit(130);
}

/// Waits the duration in the control file, for a test that needs the step to run.
fn sleep_for(path: &str) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    if let Ok(millis) = text.trim().parse::<u64>() {
        std::thread::sleep(Duration::from_millis(millis));
    }
}

/// Appends the names of the rest-server credential variables that hold a value.
fn record_credential_names(path: &str) {
    let names: Vec<String> = ["RESTIC_REST_USERNAME", "RESTIC_REST_PASSWORD"]
        .into_iter()
        .filter(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        .map(str::to_owned)
        .collect();
    if !names.is_empty() {
        record(path, &names);
    }
}

/// Appends one line with the arguments of this run. A failure opens no file, and the test then sees
/// no line.
fn record(path: &str, args: &[String]) {
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let line = format!("{}\n", args.join(" "));
    let _ = file.write_all(line.as_bytes());
}
