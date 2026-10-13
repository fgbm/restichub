#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_restichub-canary-scan"))
        .args(args)
        .output()
        .expect("the tool must start")
}

fn temp_file(name: &str, content: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "restichub-canary-cli-{name}-{}",
        std::process::id()
    ));
    std::fs::write(&path, content).expect("the file must be written");
    path
}

#[test]
fn a_clean_target_exits_zero() {
    let path = temp_file("clean", "the log holds no marker\n");
    let output = run(&[
        "--canary",
        "CANARY-EXIT-1",
        "--file",
        &path.to_string_lossy(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_finding_exits_one_and_the_output_hides_the_canary() {
    let path = temp_file("found", "prefix CANARY-EXIT-1 suffix\n");
    let output = run(&[
        "--canary",
        "CANARY-EXIT-1",
        "--file",
        &path.to_string_lossy(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("encoding=raw"), "{text}");
    assert!(text.contains("[REDACTED]"), "{text}");
    assert!(!text.contains("CANARY-EXIT-1"), "{text}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_usage_error_exits_two() {
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("canary"), "{text}");
}
