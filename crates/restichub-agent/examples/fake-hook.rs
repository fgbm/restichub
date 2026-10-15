//! A scripted hook program for the agent hook tests (FR-HK-03, FR-HK-05, AC-HK-04, AC-HK-06).
//!
//! The allow-list gives the double a first fixed argument `--control=<file>`. The control file
//! holds one command in each line:
//!
//! - `exit <code>` ends the program with the code.
//! - `sleep <ms>` waits.
//! - `out <text>` and `err <text>` print a line to the standard output or the standard error.
//! - `env <NAME>` prints the value of an environment variable to the standard output.
//! - `child` starts a grandchild that sleeps for a minute and writes its process identifier to
//!   `<control>.child`, so a test proves that a stop kills the whole tree.
//! - `ignore-interrupt` ignores the graceful stop signal.
//!
//! The double appends its arguments and its whole environment to `<control>.record`, so a test
//! proves that a value is one argument, that a secret is in the environment only and that nothing
//! of the agent environment arrives. The program is a plain example, so the Linux and the Windows
//! runner build it.

use std::io::Write as _;
use std::time::Duration;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|argument| argument == "--sleep-forever")
    {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    let Some(control) = args
        .first()
        .and_then(|argument| argument.strip_prefix("--control="))
        .map(str::to_owned)
    else {
        std::process::exit(64);
    };
    args.remove(0);
    record(&format!("{control}.record"), &args);
    let script = std::fs::read_to_string(&control).unwrap_or_default();
    let mut code = 0;
    for line in script.lines() {
        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
        match command {
            "exit" => code = rest.trim().parse().unwrap_or(1),
            "sleep" => {
                if let Ok(millis) = rest.trim().parse::<u64>() {
                    std::thread::sleep(Duration::from_millis(millis));
                }
            }
            "out" => print_line(false, rest),
            "err" => print_line(true, rest),
            "env" => print_line(false, &std::env::var(rest.trim()).unwrap_or_default()),
            "child" => start_child(&format!("{control}.child")),
            "ignore-interrupt" => ignore_interrupt(),
            _ => {}
        }
    }
    std::process::exit(code);
}

fn print_line(to_error: bool, text: &str) {
    if to_error {
        let mut stream = std::io::stderr().lock();
        let _ = writeln!(stream, "{text}");
        let _ = stream.flush();
    } else {
        let mut stream = std::io::stdout().lock();
        let _ = writeln!(stream, "{text}");
        let _ = stream.flush();
    }
}

/// Appends one block with the arguments and the environment of this run.
fn record(path: &str, args: &[String]) {
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "run");
    for argument in args {
        let _ = writeln!(file, "arg:{argument}");
    }
    let mut variables: Vec<(String, String)> = std::env::vars().collect();
    variables.sort();
    for (name, value) in variables {
        let _ = writeln!(file, "env:{name}={value}");
    }
    let _ = writeln!(
        file,
        "cwd:{}",
        std::env::current_dir().map_or_else(|_| String::new(), |dir| dir.display().to_string())
    );
}

fn start_child(path: &str) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Ok(child) = std::process::Command::new(exe)
        .arg("--sleep-forever")
        .spawn()
    {
        let _ = std::fs::write(path, child.id().to_string());
    }
}

#[allow(unsafe_code)]
fn ignore_interrupt() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `SIG_IGN` is a valid disposition for `SIGINT`.
        unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

        // A null handler with TRUE makes the process ignore Ctrl+C. A CTRL_BREAK event still ends
        // it, so the handler below ignores that event as well.
        unsafe extern "system" fn handler(_kind: u32) -> i32 {
            1
        }
        // SAFETY: the handler is a plain function that lives for the whole process.
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }
}
