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
//! - `<location>.restichub-ignore` makes the double ignore `SIGINT` on Linux, so a test covers the
//!   forced kill of the process tree (FR-AG-10, AC-AG-07).
//!
//! The binary is a plain example, so both the Linux and the Windows runner build it and the agent's
//! hash check accepts it, as in production (ADR-0031).

use std::io::Write as _;
use std::time::Duration;

fn main() {
    let repository = std::env::var("RESTIC_REPOSITORY").unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    record(&format!("{repository}.restichub-argv"), &args);
    ignore_interrupt(&format!("{repository}.restichub-ignore"));
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
    let code = if args.first().map(String::as_str) == Some("init") {
        init
    } else {
        test_access
    };
    std::process::exit(code);
}

/// Ignores `SIGINT` when the marker file asks for it, so only the forced kill stops the double.
#[allow(unsafe_code)]
fn ignore_interrupt(path: &str) {
    if !std::fs::read_to_string(path).is_ok_and(|value| !value.trim().is_empty()) {
        return;
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `signal` installs the standard ignore handler for SIGINT.
        unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    }
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
