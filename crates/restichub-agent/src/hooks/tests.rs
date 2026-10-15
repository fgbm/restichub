#![allow(clippy::expect_used)]

use std::os::unix::fs::PermissionsExt as _;

use restichub_proto::agent::v1::{
    HookParameter, HookPhase as ProtoPhase, HookStep, hook_parameter::Typed,
};

use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "restichub-agent-hooks-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
        .expect("cannot set the mode");
    dir.canonicalize().expect("cannot resolve the directory")
}

fn store(dir: &Path, text: &str, mode: u32) -> HookStore {
    let path = dir.join("hooks.toml");
    std::fs::write(&path, text).expect("cannot write the file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
        .expect("cannot set the mode");
    HookStore::new(path, Trust::CurrentUser)
}

fn executable(dir: &Path) -> PathBuf {
    let path = dir.join("tool");
    std::fs::write(&path, "#!/bin/sh\n").expect("cannot write the program");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("cannot set the mode");
    path
}

const BEFORE: i32 = ProtoPhase::Before as i32;

fn string(name: &str, value: &str) -> HookParameter {
    HookParameter {
        name: name.to_owned(),
        typed: Some(Typed::StringValue(value.to_owned())),
        ..HookParameter::default()
    }
}

fn step(hook_id: &str, phase: i32, parameters: Vec<HookParameter>) -> HookStep {
    HookStep {
        hook_id: hook_id.to_owned(),
        phase,
        parameters,
    }
}

fn text(dir: &Path) -> String {
    format!(
        "[[hook]]\nid = 'dump'\nexecutable = '{}'\nargs = ['--fixed']\n\n[[hook.parameter]]\nname = 'database'\ntype = 'string'\nmax_length = 16\npattern = '[a-z]+'\nrequired = false\n\n[[hook.parameter]]\nname = 'target'\ntype = 'string'\npass_as = 'positional'\nposition = 1\n",
        executable(dir).display()
    )
}

#[test]
fn fr_hk_03_the_arguments_are_the_fixed_ones_then_the_flags_then_the_positionals() {
    let dir = temp_dir("order");
    let list = store(&dir, &text(&dir), 0o644).load();
    assert_eq!(list.state(), AllowListState::Loaded);
    let invocation = list
        .prepare(&step(
            "dump",
            BEFORE,
            vec![string("target", "out.sql"), string("database", "app")],
        ))
        .expect("the step obeys the schema");
    assert_eq!(invocation.args, ["--fixed", "--database=app", "out.sql"]);
    assert!(invocation.env.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_hk_04_the_agent_builds_no_shell_string() {
    let dir = temp_dir("literal");
    let list = store(&dir, &text(&dir), 0o644).load();
    for value in ["; rm -rf /", "$(id)", "`id`", "a && b", "x y"] {
        let invocation = list
            .prepare(&step("dump", BEFORE, vec![string("target", value)]))
            .expect("a free text is one argument");
        assert_eq!(invocation.args, ["--fixed", value]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_hk_03_an_unknown_hook_and_a_broken_file_are_refused() {
    let dir = temp_dir("refuse");
    let list = store(&dir, &text(&dir), 0o644).load();
    assert_eq!(
        list.prepare(&step("other", BEFORE, vec![])).err(),
        Some(Refusal::Unknown)
    );
    let broken = store(&dir, "this is not toml = = =", 0o644).load();
    assert_eq!(broken.state(), AllowListState::Invalid);
    assert_eq!(
        broken.prepare(&step("dump", BEFORE, vec![])).err(),
        Some(Refusal::Insecure)
    );
    let missing = HookStore::new(dir.join("missing.toml"), Trust::CurrentUser).load();
    assert_eq!(missing.state(), AllowListState::Absent);
    assert_eq!(
        missing.prepare(&step("dump", BEFORE, vec![])).err(),
        Some(Refusal::Unknown)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_hk_01_a_file_larger_than_the_limit_is_invalid() {
    let dir = temp_dir("large");
    let big = format!("# {}\n", "x".repeat(300 * 1024));
    assert_eq!(
        store(&dir, &big, 0o644).load().state(),
        AllowListState::Invalid
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fr_hk_02_the_allow_list_file_is_a_sibling_of_the_configuration_directory() {
    assert_eq!(
        allow_list_file_for(Path::new("/etc/restichub-agent")),
        PathBuf::from("/etc/restichub-agent-hooks.toml")
    );
}
