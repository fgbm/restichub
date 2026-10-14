//! A scripted restic double for the agent job tests.
//!
//! The agent runs a step with the repository location in `RESTIC_REPOSITORY`. The double reads a
//! sibling file `<location>.restichub-fake` that holds two exit codes: the `test_access` code and
//! the `init` code. It appends its own arguments to `<location>.restichub-argv`, so a test can prove
//! that no repository secret reaches an argument (AC-AG-04). It exits with the code of the step.
//!
//! Two optional sibling files change the timing:
//!
//! - `<location>.restichub-sleep` holds a duration in milliseconds; the double waits it before it
//!   exits, so a test can drop the stream or cancel the job while the step runs.
//! - `<location>.restichub-ignore` makes the double ignore `SIGINT` on Linux and `CTRL_BREAK` on
//!   Windows, so a test covers the forced kill of the process tree (FR-AG-10, AC-AG-07). Without
//!   it the double exits with code 130 on `CTRL_BREAK`.
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
    record_priority(&format!("{repository}.restichub-state"));
    ignore_interrupt(&format!("{repository}.restichub-ignore"));
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

/// Ignores the interrupt when the marker file asks for it, so only the forced kill stops the double.
///
/// On Windows the double handles `CTRL_BREAK`: it exits with code 130 as restic does, or ignores the
/// event when the marker asks for it.
#[allow(unsafe_code)]
fn ignore_interrupt(path: &str) {
    let ignore = std::fs::read_to_string(path).is_ok_and(|value| !value.trim().is_empty());
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
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = ignore;
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
