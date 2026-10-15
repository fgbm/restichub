use std::collections::BTreeMap;

use proptest::prelude::*;
use restichub_proto::agent::v1::{Job, RepositoryAccess, step};
use time::{Date, Month, OffsetDateTime};

use super::{Plan, compile};
use crate::arg;
use crate::error::CompileError;
use crate::job::ValidatedJob;
use crate::option::ResticOption;
use crate::preview::preview;
use crate::profile::{
    BackupOptions, CheckOptions, HookPhase, Profile, ProfilePaths, Retention, SnapshotFilter,
};
use crate::target::{Architecture, OperatingSystem, ResticVersion, TargetFacts};

const BAD_OPTION: &str = "--password-command=cmd /c calc";
const BAD_SHORT: &str = "-x";
const PROFILE_TAG: &str = "restichub-profile-test";

fn date(year: i32, month: u8, day: u8) -> OffsetDateTime {
    let month = Month::try_from(month).unwrap_or(Month::January);
    match Date::from_calendar_date(year, month, day) {
        Ok(value) => value.midnight().assume_utc(),
        Err(_) => panic!("invalid test date"),
    }
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
        server_scripts: crate::script::ServerScripts::default(),
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
        profile_tag: Some(PROFILE_TAG.to_owned()),
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

fn validated(plan: &Plan, repository: &RepositoryAccess) -> ValidatedJob {
    let job = Job {
        job_id: "job-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        repositories: vec![repository.clone()],
        steps: plan
            .steps
            .iter()
            .map(|planned| planned.step.clone())
            .collect(),
        ..Job::default()
    };
    match ValidatedJob::new(job) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
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
            Some(step::Kind::Backup(_)) => {
                assert!(!planned.maintenance);
            }
            Some(step::Kind::Forget(_))
            | Some(step::Kind::Prune(_))
            | Some(step::Kind::Check(_)) => assert!(planned.maintenance),
            other => panic!("unexpected step {other:?}"),
        }
    }
}

fn forget_filter(plan: &Plan) -> restichub_proto::agent::v1::SnapshotsFilter {
    plan.steps
        .iter()
        .find_map(|planned| match &planned.step.kind {
            Some(step::Kind::Forget(forget)) => forget.filter.clone(),
            _ => None,
        })
        .expect("a forget step with a filter")
}

#[test]
fn fr_rp_08_the_forget_carries_the_profile_tag_and_the_filter_tags_as_one_and_list() {
    let mut profile = profile();
    profile.retention = Some(Retention {
        keep_last: 1,
        filter: SnapshotFilter {
            tags: vec!["daily".to_owned()],
            ..SnapshotFilter::default()
        },
        ..Retention::default()
    });
    let plan = plan(&profile, &target());
    let filter = forget_filter(&plan);
    assert_eq!(filter.all_tags, [PROFILE_TAG, "daily"]);
    assert!(filter.tags.is_empty());

    let preview = preview(&plan, &repository(), OperatingSystem::Linux).expect("the preview");
    let forget = preview
        .iter()
        .find(|step| matches!(step.step.kind, Some(step::Kind::Forget(_))))
        .expect("a forget step");
    assert!(
        forget
            .command
            .contains(&format!("--tag={PROFILE_TAG},daily")),
        "{}",
        forget.command
    );
}

#[test]
fn ac_pf_01_the_preview_shows_the_argv_that_the_agent_runs_in_the_quoting_of_the_os() {
    let mut profile = profile();
    profile.paths.all = vec!["C:\\My Data".to_owned()];
    let plan = plan(&profile, &target());
    let command_on = |os| {
        preview(&plan, &repository(), os)
            .expect("the preview")
            .into_iter()
            .find(|step| matches!(step.step.kind, Some(step::Kind::Backup(_))))
            .expect("a backup step")
            .command
    };
    let linux = command_on(OperatingSystem::Linux);
    assert!(linux.starts_with("restic backup --json "), "{linux}");
    assert!(linux.contains("'C:\\My Data'"), "{linux}");
    let windows = command_on(OperatingSystem::Windows);
    assert!(windows.contains("\"C:\\My Data\""), "{windows}");
}

#[test]
fn fr_rp_08_the_forget_without_filter_tags_filters_on_the_profile_tag() {
    let mut profile = profile();
    profile.retention = Some(Retention {
        keep_last: 1,
        ..Retention::default()
    });
    assert_eq!(
        forget_filter(&plan(&profile, &target())).all_tags,
        [PROFILE_TAG]
    );

    profile.profile_tag = None;
    assert!(
        matches!(
            error(&profile, &target()),
            CompileError::StepsRejected { .. }
        ),
        "a forget without any tag is not a valid step"
    );
}

#[test]
fn fr_rp_08_a_tag_that_expands_to_the_reserved_prefix_fails_to_compile() {
    let mut facts = target();
    facts.agent_name = "ResticHub-profile-other".to_owned();
    let template = "{{agent.name}}";

    let mut tagged = profile();
    tagged.tags = vec![template.to_owned()];
    assert!(matches!(
        error(&tagged, &facts),
        CompileError::InvalidValue { .. }
    ));

    let mut filtered = profile();
    filtered.retention = Some(Retention {
        keep_last: 1,
        filter: SnapshotFilter {
            tags: vec![template.to_owned()],
            ..SnapshotFilter::default()
        },
        ..Retention::default()
    });
    assert!(matches!(
        error(&filtered, &facts),
        CompileError::InvalidValue { .. }
    ));

    let mut keep = profile();
    keep.retention = Some(Retention {
        keep_last: 1,
        keep_tags: vec![template.to_owned()],
        ..Retention::default()
    });
    assert!(matches!(
        error(&keep, &facts),
        CompileError::InvalidValue { .. }
    ));
}

#[test]
fn fr_rp_08_the_backup_carries_the_profile_tag() {
    let plan = plan(&profile(), &target());
    let Some(step::Kind::Backup(backup)) = &plan.steps[0].step.kind else {
        panic!("a backup step");
    };
    assert_eq!(backup.tags, ["daily", PROFILE_TAG]);
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
fn ac_ag_12_the_plan_carries_the_job_limits() {
    let mut profile = profile();
    profile.limits = crate::profile::JobLimits {
        stall_timeout_seconds: 1800,
        max_duration_seconds: 86_400,
    };
    assert_eq!(plan(&profile, &target()).limits, profile.limits);
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
    let plan = plan(&profile, &target());
    let validated = validated(&plan, &repository());
    let step = match &plan.steps[0].step.kind {
        Some(step::Kind::Backup(backup)) => step::Kind::Backup(backup.clone()),
        other => panic!("unexpected step {other:?}"),
    };
    let step = restichub_proto::agent::v1::Step {
        kind: Some(step),
        ..restichub_proto::agent::v1::Step::default()
    };
    let line = match arg::build_command(&validated, &step) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    };

    let mut positional = false;
    for (index, argument) in line.args().iter().enumerate() {
        if argument == "--" {
            positional = true;
            continue;
        }
        if positional {
            continue;
        }
        assert!(index == 0 || argument.starts_with("--"), "{argument}");
    }
    assert!(line.args().contains(&BAD_OPTION.to_owned()));
    assert!(line.args().contains(&BAD_SHORT.to_owned()));
    assert!(line.args().contains(&format!("--exclude={BAD_OPTION}")));
    assert!(line.args().contains(&format!("--tag={BAD_SHORT}")));
}

#[test]
fn ac_pr_08_rejects_empty_or_comma_tag() {
    for bad in ["", ",", "a,b", " daily", "daily ", "a\tb"] {
        let mut tagged = profile();
        tagged.tags = vec![bad.to_owned()];
        assert!(
            matches!(error(&tagged, &target()), CompileError::InvalidValue { .. }),
            "{bad:?}"
        );
    }

    for bad in ["", ",", " keep"] {
        let mut keep = profile();
        keep.retention = Some(Retention {
            keep_tags: vec![bad.to_owned()],
            ..Retention::default()
        });
        assert!(
            matches!(error(&keep, &target()), CompileError::InvalidValue { .. }),
            "{bad:?}"
        );

        let mut filter = profile();
        filter.retention = Some(Retention {
            filter: SnapshotFilter {
                tags: vec![bad.to_owned()],
                ..SnapshotFilter::default()
            },
            ..Retention::default()
        });
        assert!(
            matches!(error(&filter, &target()), CompileError::InvalidValue { .. }),
            "{bad:?}"
        );
    }
}

#[test]
fn ac_pr_08_rejects_format_characters() {
    for bad in [
        "a\u{202E}",
        "a\u{200B}",
        "a\u{2028}",
        "a\u{2029}",
        "a\u{001B}",
        "a\u{007F}",
    ] {
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
    let previews = match preview(&plan, &repository(), OperatingSystem::Linux) {
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

fn attack() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just(",".to_owned()),
        Just("a,b".to_owned()),
        Just("-x".to_owned()),
        Just("--".to_owned()),
        Just("--password-command=cmd /c calc".to_owned()),
        Just("\u{202E}".to_owned()),
        Just("\u{FF0D}".to_owned()),
        Just("{{os}}".to_owned()),
        Just("%n".to_owned()),
        any::<String>(),
    ]
}

proptest! {
    #[test]
    fn injection_values_never_become_options(
        values in prop::collection::vec(attack(), 0..6),
    ) {
        let profile = Profile {
            repository_id: "repo-1".to_owned(),
            paths: ProfilePaths {
                all: values.clone(),
                by_os: BTreeMap::new(),
            },
            excludes: values.clone(),
            exclude_files: values.clone(),
            tags: values.clone(),
            profile_tag: Some(PROFILE_TAG.to_owned()),
            retention: Some(Retention {
                keep_last: 1,
                keep_tags: values.clone(),
                filter: SnapshotFilter {
                    tags: values.clone(),
                    ..SnapshotFilter::default()
                },
                ..Retention::default()
            }),
            check: Some(CheckOptions {
                read_data_subset: "10%".to_owned(),
            }),
            ..Profile::default()
        };

        let Ok(plan) = compile(&profile, &target()) else {
            return Ok(());
        };
        let validated = match ValidatedJob::new(Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![repository()],
            steps: plan.steps.iter().map(|planned| planned.step.clone()).collect(),
            ..Job::default()
        }) {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        };

        for planned in &plan.steps {
            match &planned.step.kind {
                Some(step::Kind::Backup(backup)) => {
                    for tag in &backup.tags {
                        prop_assert!(!tag.is_empty() && !tag.contains(',') && tag.trim() == tag);
                    }
                }
                Some(step::Kind::Forget(forget)) => {
                    for tag in &forget.keep_tags {
                        prop_assert!(!tag.is_empty() && !tag.contains(',') && tag.trim() == tag);
                    }
                    if let Some(filter) = &forget.filter {
                        for tag in filter.tags.iter().chain(&filter.all_tags) {
                            prop_assert!(!tag.is_empty() && !tag.contains(',') && tag.trim() == tag);
                        }
                    }
                }
                _ => {}
            }

            let Ok(line) = arg::build_command(&validated, &planned.step) else {
                continue;
            };
            let mut positional = false;
            for (index, argument) in line.args().iter().enumerate() {
                prop_assert!(!restichub_proto::validate::has_forbidden(argument));
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

fn script(name: &str, phase: HookPhase) -> crate::script::ScriptRef {
    crate::script::ScriptRef {
        name: name.to_owned(),
        interpreter: crate::script::ScriptInterpreter::Bash,
        phase,
        body: "echo ok".to_owned(),
        timeout_seconds: 60,
        failure_policy: crate::script::FailurePolicy::AbortBackup,
        env: Vec::new(),
    }
}

fn trusting_target() -> TargetFacts {
    let mut target = target();
    target.server_scripts = crate::script::ServerScripts {
        enabled: true,
        interpreters: vec![crate::script::ScriptInterpreter::Bash],
    };
    target
}

#[test]
fn ac_hk_07_a_script_for_a_host_without_the_trust_mode_is_refused() {
    let mut profile = profile();
    profile.scripts = vec![script("dump", HookPhase::Before)];
    let error = compile(&profile, &target()).expect_err("the host did not turn on the mode");
    assert!(matches!(error, CompileError::ServerScriptsDisabled { .. }));
    assert!(error.is_script());
}

#[test]
fn fr_hk_10_a_script_needs_an_interpreter_of_the_host() {
    let mut profile = profile();
    let mut powershell = script("dump", HookPhase::Before);
    powershell.interpreter = crate::script::ScriptInterpreter::Powershell;
    profile.scripts = vec![powershell];
    let error = compile(&profile, &trusting_target()).expect_err("no powershell on this host");
    assert!(matches!(error, CompileError::InterpreterUnavailable { .. }));
}

#[test]
fn fr_hk_11_the_scripts_are_steps_in_the_phase_order() {
    let mut profile = profile();
    profile.scripts = vec![
        script("late", HookPhase::Finally),
        script("first", HookPhase::Before),
        script("second", HookPhase::Before),
        script("after", HookPhase::After),
        script("failed", HookPhase::OnError),
    ];
    let plan = plan(&profile, &trusting_target());
    let order: Vec<String> = plan
        .steps
        .iter()
        .map(|planned| match &planned.step.kind {
            Some(step::Kind::Script(script)) => script.name.clone(),
            Some(step::Kind::Backup(_)) => "backup".to_owned(),
            _ => "other".to_owned(),
        })
        .collect();
    assert_eq!(
        order,
        ["first", "second", "backup", "after", "failed", "late"]
    );
    let ids: Vec<&str> = plan
        .steps
        .iter()
        .map(|planned| planned.step.step_id.as_str())
        .collect();
    assert_eq!(ids[0], "script-1");
    assert_eq!(ids[1], "script-2");
}

#[test]
fn ac_pf_01_the_preview_of_a_script_shows_no_body_and_no_value() {
    let mut profile = profile();
    let mut dump = script("dump", HookPhase::Before);
    dump.body = "echo CANARY-BODY".to_owned();
    dump.env = vec![crate::script::ScriptEnvRef {
        name: "PGPASSWORD".to_owned(),
        value: crate::script::ScriptEnvValue::Secret("CANARY-SECRET".to_owned()),
    }];
    profile.scripts = vec![dump];
    let plan = plan(&profile, &trusting_target());
    let lines = preview(&plan, &repository(), OperatingSystem::Linux).expect("preview");
    let line = &lines[0].command;
    assert!(line.contains("script=dump"));
    assert!(line.contains("PGPASSWORD"));
    assert!(!line.contains("CANARY"));
}
