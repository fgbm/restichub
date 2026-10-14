use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use restichub_proto::agent::v1::{RepositoryAccess, step};
use time::{Date, Month, OffsetDateTime};

use super::{Plan, compile};
use crate::arg;
use crate::error::CompileError;
use crate::option::ResticOption;
use crate::preview::preview;
use crate::profile::{
    BackupOptions, CheckOptions, HookParameter, HookPhase, HookRef, Profile, ProfilePaths,
    Retention,
};
use crate::target::{AllowListedHook, Architecture, OperatingSystem, ResticVersion, TargetFacts};

const BAD_OPTION: &str = "--password-command=cmd /c calc";
const BAD_SHORT: &str = "-x";

fn date(year: i32, month: u8, day: u8) -> OffsetDateTime {
    let month = Month::try_from(month).unwrap_or(Month::January);
    match Date::from_calendar_date(year, month, day) {
        Ok(value) => value.midnight().assume_utc(),
        Err(_) => panic!("invalid test date"),
    }
}

fn allow_list() -> BTreeMap<String, AllowListedHook> {
    let mut allow_list = BTreeMap::new();
    allow_list.insert(
        "pg_dump".to_owned(),
        AllowListedHook {
            parameters: ["database".to_owned()].into_iter().collect::<BTreeSet<_>>(),
        },
    );
    allow_list
}

fn target() -> TargetFacts {
    let mut labels = BTreeMap::new();
    labels.insert("env".to_owned(), "prod".to_owned());
    TargetFacts {
        agent_id: "agent-1".to_owned(),
        agent_name: "web-1".to_owned(),
        hostname: "host1".to_owned(),
        os: OperatingSystem::Linux,
        arch: Architecture::X86_64,
        labels,
        restic_version: ResticVersion::MIN_SUPPORTED,
        now: date(2026, 10, 13),
        allow_list: allow_list(),
    }
}

fn profile() -> Profile {
    Profile {
        repository_id: "repo-1".to_owned(),
        paths: ProfilePaths {
            all: vec!["/srv".to_owned()],
            by_os: BTreeMap::new(),
        },
        tags: vec!["daily".to_owned()],
        ..Profile::default()
    }
}

fn repository() -> RepositoryAccess {
    RepositoryAccess {
        repository_id: "repo-1".to_owned(),
        location: "s3:s3.example.com/bucket".to_owned(),
        password: "topsecret".to_owned(),
        ..RepositoryAccess::default()
    }
}

fn plan(profile: &Profile, target: &TargetFacts) -> Plan {
    match compile(profile, target) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    }
}

fn error(profile: &Profile, target: &TargetFacts) -> CompileError {
    match compile(profile, target) {
        Ok(value) => panic!("expected an error, got {value:?}"),
        Err(error) => error,
    }
}

fn backup_paths(plan: &Plan) -> Vec<String> {
    for planned in &plan.steps {
        if let Some(step::Kind::Backup(backup)) = &planned.step.kind {
            return backup.paths.clone();
        }
    }
    panic!("no backup step");
}

fn hook(hook_id: &str, phase: HookPhase, parameters: &[(&str, &str)]) -> HookRef {
    HookRef {
        hook_id: hook_id.to_owned(),
        phase,
        parameters: parameters
            .iter()
            .map(|(name, value)| HookParameter {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
    }
}

#[test]
fn backup_step_is_built_with_retry_lock() {
    let plan = plan(&profile(), &target());
    assert_eq!(plan.steps.len(), 1);
    let planned = &plan.steps[0];
    assert!(!planned.maintenance);
    match &planned.step.kind {
        Some(step::Kind::Backup(backup)) => {
            assert!(backup.retry_lock);
            assert_eq!(backup.repository_id, "repo-1");
            assert_eq!(backup.paths, vec!["/srv".to_owned()]);
        }
        other => panic!("unexpected step {other:?}"),
    }
}

#[test]
fn retention_and_check_add_maintenance_steps() {
    let mut profile = profile();
    profile.retention = Some(Retention::default());
    profile.check = Some(CheckOptions {
        read_data_subset: "10%".to_owned(),
    });
    let plan = plan(&profile, &target());
    assert_eq!(plan.steps.len(), 4);
    for planned in &plan.steps {
        match &planned.step.kind {
            Some(step::Kind::Backup(_)) | Some(step::Kind::Hook(_)) => {
                assert!(!planned.maintenance);
            }
            Some(step::Kind::Forget(_))
            | Some(step::Kind::Prune(_))
            | Some(step::Kind::Check(_)) => assert!(planned.maintenance),
            other => panic!("unexpected step {other:?}"),
        }
    }
}

#[test]
fn before_hook_runs_before_the_backup() {
    let mut profile = profile();
    profile.hooks = vec![hook("pg_dump", HookPhase::Before, &[("database", "app")])];
    let plan = plan(&profile, &target());
    assert_eq!(plan.steps.len(), 2);
    assert!(matches!(plan.steps[0].step.kind, Some(step::Kind::Hook(_))));
    assert!(matches!(
        plan.steps[1].step.kind,
        Some(step::Kind::Backup(_))
    ));
}

#[test]
fn ac_pf_02_vss_on_linux_agent_is_rejected() {
    let mut profile = profile();
    profile.options = BackupOptions {
        use_fs_snapshot: true,
        ..BackupOptions::default()
    };
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::UnsupportedOs {
            option: ResticOption::UseFsSnapshot,
            os: OperatingSystem::Linux,
            ..
        }
    ));
}

#[test]
fn ac_pf_02_vss_on_windows_agent_is_accepted() {
    let mut profile = profile();
    profile.options = BackupOptions {
        use_fs_snapshot: true,
        ..BackupOptions::default()
    };
    let mut target = target();
    target.os = OperatingSystem::Windows;
    assert!(compile(&profile, &target).is_ok());
}

#[test]
fn ac_pf_02_missing_hook_is_rejected() {
    let mut profile = profile();
    profile.hooks = vec![hook("missing", HookPhase::Before, &[])];
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::UnknownHook { hook_id, .. } if hook_id == "missing"
    ));
}

#[test]
fn ac_pf_02_unknown_hook_parameter_is_rejected() {
    let mut profile = profile();
    profile.hooks = vec![hook("pg_dump", HookPhase::Before, &[("other", "app")])];
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::UnknownHookParameter { name, .. } if name == "other"
    ));
}

#[test]
fn duplicate_hook_parameter_is_rejected() {
    let mut profile = profile();
    profile.hooks = vec![hook(
        "pg_dump",
        HookPhase::Before,
        &[("database", "app"), ("database", "other")],
    )];
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::DuplicateHookParameter { name, .. } if name == "database"
    ));
}

#[test]
fn ac_pf_03_variables_give_each_agent_its_own_values() {
    let mut profile = profile();
    profile.paths.all = vec!["/srv/{{hostname}}/{{os}}/{{agent.label.env}}".to_owned()];

    let linux = target();
    let mut windows = target();
    windows.os = OperatingSystem::Windows;
    windows.hostname = "host2".to_owned();
    windows.labels.insert("env".to_owned(), "dev".to_owned());
    let mut macos = target();
    macos.os = OperatingSystem::Macos;
    macos.hostname = "host3".to_owned();

    assert_eq!(
        backup_paths(&plan(&profile, &linux)),
        vec!["/srv/host1/linux/prod".to_owned()]
    );
    assert_eq!(
        backup_paths(&plan(&profile, &windows)),
        vec!["/srv/host2/windows/dev".to_owned()]
    );
    assert_eq!(
        backup_paths(&plan(&profile, &macos)),
        vec!["/srv/host3/macos/prod".to_owned()]
    );
}

#[test]
fn ac_pr_08_injected_values_stay_out_of_option_positions() {
    let mut profile = profile();
    profile.paths.all = vec![BAD_OPTION.to_owned(), BAD_SHORT.to_owned()];
    profile.excludes = vec![BAD_OPTION.to_owned()];
    profile.tags = vec![BAD_SHORT.to_owned()];
    let backup = plan(&profile, &target())
        .steps
        .into_iter()
        .find_map(|planned| match planned.step.kind {
            Some(step::Kind::Backup(backup)) => Some(backup),
            _ => None,
        });
    let backup = match backup {
        Some(value) => value,
        None => panic!("no backup step"),
    };
    let step = restichub_proto::agent::v1::Step {
        kind: Some(step::Kind::Backup(backup)),
        ..restichub_proto::agent::v1::Step::default()
    };
    let line = match arg::build_command(&step, &repository()) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    };

    let mut positional = false;
    for (index, argument) in line.args.iter().enumerate() {
        if argument == "--" {
            positional = true;
            continue;
        }
        if positional {
            continue;
        }
        assert!(index == 0 || argument.starts_with("--"), "{argument}");
    }
    assert!(line.args.contains(&BAD_OPTION.to_owned()));
    assert!(line.args.contains(&BAD_SHORT.to_owned()));
    assert!(line.args.contains(&format!("--exclude={BAD_OPTION}")));
    assert!(line.args.contains(&format!("--tag={BAD_SHORT}")));
}

#[test]
fn ac_pr_08_newline_and_nul_values_are_rejected() {
    for bad in ["bad\nvalue", "bad\rvalue", "bad\0value"] {
        let mut profile = profile();
        profile.paths.all = vec![bad.to_owned()];
        assert!(
            matches!(
                error(&profile, &target()),
                CompileError::InvalidValue { .. }
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn unterminated_template_is_rejected() {
    let mut profile = profile();
    profile.paths.all = vec!["{{hostname".to_owned()];
    assert!(matches!(
        error(&profile, &target()),
        CompileError::UnterminatedTemplate { .. }
    ));
}

#[test]
fn missing_label_is_rejected() {
    let mut profile = profile();
    profile.paths.all = vec!["{{agent.label.nope}}".to_owned()];
    assert!(matches!(
        error(&profile, &target()),
        CompileError::MissingLabel { .. }
    ));
}

#[test]
fn empty_paths_are_rejected() {
    let mut profile = profile();
    profile.paths.all = Vec::new();
    assert!(matches!(
        error(&profile, &target()),
        CompileError::NoPaths { .. }
    ));
}

#[test]
fn ac_pf_01_preview_masks_secrets() {
    let plan = plan(&profile(), &target());
    let previews = match preview(&plan, &repository()) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    };
    let backup = &previews[0];
    assert!(backup.command.starts_with("restic backup"));
    assert!(!backup.command.contains("topsecret"));
    assert!(
        backup
            .environment
            .iter()
            .any(|(name, value)| name == "RESTIC_PASSWORD" && value == "***")
    );
}

proptest! {
    #[test]
    fn injection_values_never_become_options(values in prop::collection::vec(any::<String>(), 0..6)) {
        let profile = Profile {
            repository_id: "repo-1".to_owned(),
            paths: ProfilePaths {
                all: values.clone(),
                by_os: BTreeMap::new(),
            },
            excludes: values.clone(),
            exclude_files: values.clone(),
            tags: values.clone(),
            ..Profile::default()
        };

        let Ok(plan) = compile(&profile, &target()) else {
            return Ok(());
        };
        for planned in &plan.steps {
            let Ok(line) = arg::build_command(&planned.step, &repository()) else {
                continue;
            };
            let mut positional = false;
            for (index, argument) in line.args.iter().enumerate() {
                if argument == "--" {
                    positional = true;
                    continue;
                }
                if positional {
                    continue;
                }
                if index == 0 {
                    prop_assert!(!argument.starts_with('-'));
                    continue;
                }
                prop_assert!(argument.starts_with("--"), "value {argument:?} before --");
            }
        }
    }
}
