#![allow(clippy::expect_used)]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;

use restichub_proto::agent::v1::{
    HookPhase, RejectReason, ScriptEnv, ScriptFailurePolicy, ScriptInterpreter, ScriptStep,
    ServerScriptsState, script_env,
};

use super::setting::set_mode_at;
use super::{ScriptMode, ScriptRefusal, ScriptStore};
use crate::scrub::Scrubber;
use crate::trust::Trust;

const CANARY: &str = "CANARY-secret-7f3a";

/// A directory that the test removes at its end. The mode 0755 satisfies the trust check of the
/// current account.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-scripts-{}-{number}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("cannot set the mode");
        Self(dir.canonicalize().expect("cannot resolve the directory"))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn step(body: &str) -> ScriptStep {
    ScriptStep {
        name: "dump".to_owned(),
        interpreter: ScriptInterpreter::Bash as i32,
        phase: HookPhase::Before as i32,
        body: body.to_owned(),
        failure_policy: ScriptFailurePolicy::AbortBackup as i32,
        env: Vec::new(),
    }
}

fn store(dir: &std::path::Path) -> ScriptStore {
    ScriptStore::new(
        dir.join("restichub-agent-server-scripts"),
        Trust::CurrentUser,
        dir,
    )
}

#[test]
fn ac_hk_07_the_default_mode_is_off_and_refuses_a_script() {
    let dir = TempDir::new();
    let store = store(dir.path());
    assert_eq!(store.mode(), ScriptMode::Disabled);
    let refusal = store.check(&step("echo ok")).expect_err("the mode is off");
    assert_eq!(refusal, ScriptRefusal::Disabled);
    assert_eq!(refusal.reject_reason(), RejectReason::ServerScriptsDisabled);
    let (state, interpreters) = store.report();
    assert_eq!(state, ServerScriptsState::Disabled);
    assert!(interpreters.is_empty());
}

#[test]
fn fr_hk_08_the_setting_turns_the_mode_on_and_off() {
    let dir = TempDir::new();
    let store = store(dir.path());
    let file = dir.path().join("restichub-agent-server-scripts");
    set_mode_at(&file, true, Trust::CurrentUser).expect("enable");
    assert_eq!(store.mode(), ScriptMode::Enabled);
    let (state, interpreters) = store.report();
    assert_eq!(state, ServerScriptsState::Enabled);
    assert!(interpreters.contains(&ScriptInterpreter::Bash));
    assert!(interpreters.contains(&ScriptInterpreter::Sh));
    assert!(store.check(&step("echo ok")).is_ok());
    set_mode_at(&file, false, Trust::CurrentUser).expect("disable");
    assert_eq!(store.mode(), ScriptMode::Disabled);
}

#[test]
fn fr_hk_08_a_damaged_setting_never_turns_the_mode_on() {
    let dir = TempDir::new();
    let store = store(dir.path());
    let file = dir.path().join("restichub-agent-server-scripts");
    for text in [
        "",
        "enabled",
        "server_scripts = enable",
        "other = enabled",
        "server_scripts == enabled\nx",
    ] {
        std::fs::write(&file, text).expect("write");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("mode");
        assert_eq!(store.mode(), ScriptMode::Disabled, "{text:?}");
    }
}

#[test]
fn ac_hk_08_a_setting_that_others_can_write_turns_the_mode_off() {
    let dir = TempDir::new();
    let store = store(dir.path());
    let file = dir.path().join("restichub-agent-server-scripts");
    set_mode_at(&file, true, Trust::CurrentUser).expect("enable");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).expect("mode");
    assert_eq!(store.mode(), ScriptMode::Insecure);
    assert_eq!(
        store.check(&step("echo ok")).expect_err("not trusted"),
        ScriptRefusal::Disabled
    );
    let (state, interpreters) = store.report();
    assert_eq!(state, ServerScriptsState::Insecure);
    assert!(interpreters.is_empty());
}

#[test]
fn fr_hk_09_a_script_with_a_broken_rule_is_invalid() {
    let dir = TempDir::new();
    let store = store(dir.path());
    set_mode_at(
        &dir.path().join("restichub-agent-server-scripts"),
        true,
        Trust::CurrentUser,
    )
    .expect("enable");
    let invalid = |script: &ScriptStep| {
        let refusal = store.check(script).expect_err("invalid");
        assert_eq!(refusal.reject_reason(), RejectReason::InvalidScript);
    };
    invalid(&step("  "));
    invalid(&step(&"x".repeat(70_000)));
    invalid(&ScriptStep {
        interpreter: ScriptInterpreter::Powershell as i32,
        ..step("echo")
    });
    invalid(&ScriptStep {
        interpreter: 0,
        ..step("echo")
    });
    invalid(&ScriptStep {
        env: vec![ScriptEnv {
            name: "LD_PRELOAD".to_owned(),
            value: Some(script_env::Value::StringValue("x".to_owned())),
        }],
        ..step("echo")
    });
}

#[test]
fn ac_pf_10_a_text_parameter_value_is_masked_in_the_script_output() {
    let dir = TempDir::new();
    let store = store(dir.path());
    set_mode_at(
        &dir.path().join("restichub-agent-server-scripts"),
        true,
        Trust::CurrentUser,
    )
    .expect("enable");
    // The control plane sends a text parameter of the profile as a masked value, so the agent
    // hides it in the step log like a secret (FR-PF-12, ADR-0047).
    let script = ScriptStep {
        env: vec![ScriptEnv {
            name: "PGDIR".to_owned(),
            value: Some(script_env::Value::MaskedValue(CANARY.to_owned())),
        }],
        ..step("echo hello")
    };
    let prepared = store.prepare(&script, 30).expect("prepared");
    let invocation = &prepared.invocation;
    assert!(
        invocation
            .env
            .iter()
            .any(|(name, value)| name == "PGDIR" && value.as_str() == CANARY)
    );
    assert_eq!(invocation.secrets.len(), 1);
    assert_eq!(
        Scrubber::for_secrets(&invocation.secrets).scrub(&format!("dir {CANARY}")),
        "dir ***"
    );
}

#[test]
fn ac_hk_09_the_secret_is_only_in_the_environment_and_the_file_goes_away() {
    let dir = TempDir::new();
    let store = store(dir.path());
    set_mode_at(
        &dir.path().join("restichub-agent-server-scripts"),
        true,
        Trust::CurrentUser,
    )
    .expect("enable");
    let script = ScriptStep {
        env: vec![
            ScriptEnv {
                name: "PGPASSWORD".to_owned(),
                value: Some(script_env::Value::SecretValue(CANARY.to_owned())),
            },
            ScriptEnv {
                name: "PGHOST".to_owned(),
                value: Some(script_env::Value::StringValue("db1".to_owned())),
            },
        ],
        ..step("echo hello")
    };
    let prepared = store.prepare(&script, 30).expect("prepared");
    let invocation = &prepared.invocation;
    assert!(
        invocation
            .args
            .iter()
            .all(|argument| !argument.contains(CANARY))
    );
    assert!(
        invocation
            .env
            .iter()
            .any(|(name, value)| name == "PGPASSWORD" && value.as_str() == CANARY)
    );
    assert_eq!(invocation.secrets.len(), 1);
    assert_eq!(invocation.timeout.as_secs(), 30);
    let file = std::path::PathBuf::from(invocation.args.last().expect("file argument"));
    assert_eq!(std::fs::read_to_string(&file).expect("body"), "echo hello");
    let mode = std::fs::metadata(&file)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "the script file is private");
    let scratch = invocation.working_dir.clone();
    assert!(scratch.is_dir());
    drop(prepared);
    assert!(!file.exists());
    assert!(!scratch.exists());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("scripts"))
        .expect("scripts dir")
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn fr_hk_09_the_bash_command_line_has_the_fixed_safe_flags() {
    let dir = TempDir::new();
    let store = store(dir.path());
    set_mode_at(
        &dir.path().join("restichub-agent-server-scripts"),
        true,
        Trust::CurrentUser,
    )
    .expect("enable");
    let prepared = store.prepare(&step("echo"), 0).expect("prepared");
    assert_eq!(prepared.invocation.args[0], "--noprofile");
    assert_eq!(prepared.invocation.args[1], "--norc");
    assert_eq!(prepared.invocation.timeout, super::DEFAULT_TIMEOUT);
}

fn private_dir(parent: &std::path::Path, name: &str, mode: u32) -> std::path::PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("mode");
    dir
}

fn enabled_store(dir: &std::path::Path) -> ScriptStore {
    set_mode_at(
        &dir.join("restichub-agent-server-scripts"),
        true,
        Trust::CurrentUser,
    )
    .expect("enable");
    store(dir)
}

#[test]
fn ac_hk_08_a_search_path_directory_that_others_can_write_is_not_in_the_script_environment() {
    let dir = TempDir::new();
    let good = private_dir(dir.path(), "good", 0o755);
    let bad = private_dir(dir.path(), "bad", 0o777);
    let path = std::env::join_paths([good.as_path(), bad.as_path()]).expect("join");
    let store = enabled_store(dir.path()).with_search_path(path);
    let prepared = store.prepare(&step("echo"), 0).expect("prepared");
    let search = prepared
        .invocation
        .search_path
        .clone()
        .expect("the script gets a search path");
    let kept: Vec<_> = std::env::split_paths(&search).collect();
    assert_eq!(kept, vec![good]);
    let note = prepared.dropped_path.as_deref().expect("a note");
    assert!(note.contains(bad.to_str().expect("utf-8")));
}

#[test]
fn ac_hk_08_a_relative_or_missing_search_path_entry_is_dropped() {
    let dir = TempDir::new();
    let good = private_dir(dir.path(), "good", 0o755);
    let missing = dir.path().join("missing");
    let path = std::env::join_paths([
        std::path::PathBuf::from("relative"),
        missing.clone(),
        good.clone(),
    ])
    .expect("join");
    let store = enabled_store(dir.path()).with_search_path(path);
    let prepared = store.prepare(&step("echo"), 0).expect("prepared");
    let search = prepared.invocation.search_path.clone().expect("path");
    assert_eq!(
        std::env::split_paths(&search).collect::<Vec<_>>(),
        vec![good]
    );
    assert!(prepared.dropped_path.is_some());
}

#[test]
fn ac_hk_08_a_search_path_of_trusted_directories_drops_nothing() {
    let dir = TempDir::new();
    let first = private_dir(dir.path(), "first", 0o755);
    let second = private_dir(dir.path(), "second", 0o750);
    let path = std::env::join_paths([first.as_path(), second.as_path()]).expect("join");
    let store = enabled_store(dir.path()).with_search_path(path);
    let prepared = store.prepare(&step("echo"), 0).expect("prepared");
    assert!(prepared.dropped_path.is_none());
}

#[test]
fn ac_hk_09_a_crash_leaves_no_script_directory_after_the_next_start() {
    let dir = TempDir::new();
    let stale = dir.path().join("scripts").join("0123abcd");
    std::fs::create_dir_all(&stale).expect("cannot create the directory");
    std::fs::write(stale.join("script.sh"), "echo old").expect("cannot write the file");
    let store = store(dir.path());
    store.clean_leftovers();
    assert!(!dir.path().join("scripts").exists());
    store.clean_leftovers();
}

#[test]
fn adr_0040_an_old_allow_list_file_is_reported_by_path_only() {
    let dir = TempDir::new();
    let config = dir.path().join("restichub-agent");
    assert_eq!(super::removed_allow_list_file(&config), None);

    let file = dir.path().join("restichub-agent-hooks.toml");
    std::fs::write(&file, "secret = \"x\"").expect("cannot write the file");
    assert_eq!(super::removed_allow_list_file(&config), Some(file));
}

#[cfg(target_os = "linux")]
mod transient_unit {
    use std::os::unix::fs::PermissionsExt as _;

    use super::{CANARY, TempDir, enabled_store, step};
    use crate::scripts::{ScriptRunner, ScriptStore};
    use restichub_proto::agent::v1::{ScriptEnv, script_env};

    fn systemd_store(dir: &std::path::Path) -> ScriptStore {
        enabled_store(dir).with_runner(ScriptRunner::Systemd)
    }

    /// A value with every byte that a shell could mangle.
    const AWKWARD: &str = "a'b\"c$d\\e\nf\tg h";

    #[test]
    fn adr_0053_a_value_reaches_the_interpreter_unchanged() {
        let dir = TempDir::new();
        let store = systemd_store(dir.path());
        let script = restichub_proto::agent::v1::ScriptStep {
            env: vec![ScriptEnv {
                name: "SECRET".to_owned(),
                value: Some(script_env::Value::SecretValue(AWKWARD.to_owned())),
            }],
            ..step("echo hello")
        };
        let prepared = store.prepare(&script, 30).expect("prepared");
        let unit = prepared.invocation.unit.as_ref().expect("a transient unit");
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(crate::scripts::transient::SHELL_WRAPPER)
            .arg("restichub-script")
            .arg(&unit.env_file)
            .arg("/bin/sh")
            .arg("-c")
            .arg("printf %s \"$SECRET\"")
            .output()
            .expect("cannot run the wrapper");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), AWKWARD);
        assert!(
            !prepared
                .invocation
                .args
                .iter()
                .any(|argument| argument.contains(AWKWARD))
        );
    }

    #[test]
    fn adr_0053_the_environment_file_is_private_and_goes_away_with_the_run() {
        let dir = TempDir::new();
        let store = systemd_store(dir.path());
        let script = restichub_proto::agent::v1::ScriptStep {
            env: vec![ScriptEnv {
                name: "PGPASSWORD".to_owned(),
                value: Some(script_env::Value::SecretValue(CANARY.to_owned())),
            }],
            ..step("echo hello")
        };
        let prepared = store.prepare(&script, 30).expect("prepared");
        let unit = prepared.invocation.unit.clone().expect("a transient unit");
        assert!(unit.env_file.is_file());
        let mode = std::fs::metadata(&unit.env_file)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the environment file is private");
        let text = std::fs::read_to_string(&unit.env_file).expect("the environment file");
        assert!(text.contains(CANARY));
        assert_eq!(unit.runtime.as_secs(), 30 + 60);
        drop(prepared);
        assert!(!unit.env_file.exists());
    }

    #[test]
    fn adr_0053_the_unit_name_holds_the_random_part_of_the_run() {
        let dir = TempDir::new();
        let store = systemd_store(dir.path());
        let prepared = store.prepare(&step("echo"), 0).expect("prepared");
        let unit = prepared.invocation.unit.as_ref().expect("a transient unit");
        let random = prepared
            .invocation
            .working_dir
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("the run directory");
        assert_eq!(
            unit.name,
            format!("restichub-script-{random}.service"),
            "the unit name follows the run directory"
        );
    }

    #[test]
    fn adr_0053_the_direct_runner_writes_no_environment_file() {
        let dir = TempDir::new();
        let store = enabled_store(dir.path()).with_runner(ScriptRunner::Direct);
        let prepared = store.prepare(&step("echo"), 0).expect("prepared");
        assert!(prepared.invocation.unit.is_none());
        let entries: Vec<_> = std::fs::read_dir(&prepared.invocation.working_dir)
            .expect("the run directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert!(!entries.iter().any(|name| name == "environment.sh"));
    }

    #[test]
    fn adr_0053_a_value_with_a_nul_byte_is_refused() {
        let dir = TempDir::new();
        let store = systemd_store(dir.path());
        let script = restichub_proto::agent::v1::ScriptStep {
            env: vec![ScriptEnv {
                name: "SECRET".to_owned(),
                value: Some(script_env::Value::StringValue("a\0b".to_owned())),
            }],
            ..step("echo")
        };
        let refusal = store.check(&script).expect_err("a NUL byte is invalid");
        assert_eq!(
            refusal.reject_reason(),
            restichub_proto::agent::v1::RejectReason::InvalidScript
        );
    }
}
