//! A scripted restic double for the agent job tests.
//!
//! The agent runs a step with the repository location in `RESTIC_REPOSITORY`. The double reads a
//! sibling file `<location>.restichub-fake` that holds two exit codes: the `test_access` code and
//! the `init` code. It appends its own arguments to `<location>.restichub-argv`, so a test can prove
//! that no repository secret reaches an argument (AC-AG-04). It exits with the code of the step.
//!
//! The binary is a plain example, so both the Linux and the Windows runner build it and the agent's
//! hash check accepts it, as in production (ADR-0031).

use std::io::Write as _;

fn main() {
    let repository = std::env::var("RESTIC_REPOSITORY").unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    record(&format!("{repository}.restichub-argv"), &args);
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
