//! The hook steps against a scripted hook program, without a control plane (FR-HK-01 to FR-HK-05,
//! AC-HK-01 to AC-HK-04, AC-HK-06, ADR-0004).
//!
//! The tests drive [`Jobs::handle`] directly and read the protocol messages of the stream channel.
//! The scripted `fake-hook` example records its arguments and its environment and follows the
//! control file of the test. The allow-list file belongs to the current user, so the tests use the
//! `CurrentUser` trust; the host trust is tested with a file that the host cannot trust.

#![allow(clippy::expect_used)]

mod common;

use common::hooks::*;
use common::*;
use restichub_agent::hooks::Trust;
use restichub_agent::jobs::Jobs;
use restichub_proto::agent::v1::{
    HookAllowListState, HookParameter, HookPhase, JobStatus, RejectReason, StepStatus,
    agent_message,
};
use tokio::sync::mpsc;

const CANARY: &str = "CANARY-hook-secret-value";

/// An allow-list with one fake hook for each phase of the sequence.
fn sequence_list(fx: &HookFixture) -> String {
    ["h-before", "h-after", "h-success", "h-error", "h-finally"]
        .iter()
        .map(|id| fx.hook(id, ""))
        .collect()
}

fn sequence_job(fx: &HookFixture, job_id: &str) -> restichub_proto::agent::v1::Job {
    fx.job(
        job_id,
        vec![
            hook_step("hook-before", "h-before", HookPhase::Before, vec![]),
            backup_step("repo-1", &["/srv"]),
            hook_step("hook-after", "h-after", HookPhase::After, vec![]),
            hook_step("hook-success", "h-success", HookPhase::OnSuccess, vec![]),
            hook_step("hook-error", "h-error", HookPhase::OnError, vec![]),
            hook_step("hook-finally", "h-finally", HookPhase::Finally, vec![]),
        ],
    )
}

fn status_of(collected: &Collected, step_id: &str) -> StepStatus {
    StepStatus::try_from(collected.step(step_id).status).expect("a known status")
}

/// The reason of the first message when the agent rejects the job.
async fn rejection(jobs: &Jobs, job: &restichub_proto::agent::v1::Job) -> Option<RejectReason> {
    let (tx, mut rx) = mpsc::channel(8);
    jobs.attach(&tx);
    jobs.handle(signed(job), &ring(), AGENT, TENANT).await;
    let message = rx.recv().await.expect("the agent must answer");
    match message.payload {
        Some(agent_message::Payload::JobRejected(rejected)) => {
            Some(RejectReason::try_from(rejected.reason).expect("a known reason"))
        }
        _ => None,
    }
}

/// AC-HK-06: on success the sequence is before, the backup, after, on_success and finally.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_06_the_phases_run_in_order_on_success() {
    let fx = HookFixture::new("hook-success");
    fx.write_allow_list(&sequence_list(&fx));
    let collected = run(&fx.jobs(), &sequence_job(&fx, "job-ok")).await;
    let order: Vec<&str> = collected
        .steps
        .iter()
        .map(|step| step.step_id.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "hook-before",
            "backup",
            "hook-after",
            "hook-success",
            "hook-error",
            "hook-finally"
        ]
    );
    assert_eq!(status_of(&collected, "hook-error"), StepStatus::Skipped);
    assert_eq!(status_of(&collected, "hook-finally"), StepStatus::Success);
    assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
    assert_eq!(fx.runs("h-error"), 0);
    assert_eq!(fx.restic_runs().len(), 1);
}

/// AC-HK-06: a failed `before` hook stops the backup; `on_error` and `finally` run.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_06_a_failed_before_hook_stops_the_backup() {
    let fx = HookFixture::new("hook-before-fails");
    fx.write_allow_list(&sequence_list(&fx));
    fx.script("h-before", "err the dump failed\nexit 1\n");
    let collected = run(&fx.jobs(), &sequence_job(&fx, "job-before")).await;
    assert!(fx.restic_runs().is_empty(), "the backup must not run");
    assert_eq!(status_of(&collected, "hook-before"), StepStatus::Failed);
    assert_eq!(collected.step("hook-before").exit_code, 1);
    assert_eq!(status_of(&collected, "backup"), StepStatus::Skipped);
    assert_eq!(status_of(&collected, "hook-after"), StepStatus::Skipped);
    assert_eq!(status_of(&collected, "hook-success"), StepStatus::Skipped);
    assert_eq!(status_of(&collected, "hook-error"), StepStatus::Success);
    assert_eq!(status_of(&collected, "hook-finally"), StepStatus::Success);
    assert_eq!((fx.runs("h-after"), fx.runs("h-success")), (0, 0));
    let result = collected.result();
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    let error = result.error.as_ref().expect("a typed reason");
    assert_eq!(error.code, "hook_failed");
    let params: Vec<(&str, &str)> = error
        .params
        .iter()
        .map(|pair| (pair.key.as_str(), pair.value.as_str()))
        .collect();
    assert_eq!(params, [("hook", "h-before"), ("phase", "before")]);
    assert!(collected.log_text().contains("the dump failed"));
}

/// AC-HK-06: a failed backup runs `after`, `on_error` and `finally`, and skips `on_success`.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_06_a_failed_backup_runs_the_error_hooks() {
    let fx = HookFixture::new("hook-backup-fails");
    fx.write_allow_list(&sequence_list(&fx));
    fx.restic_code("backup", 1);
    let collected = run(&fx.jobs(), &sequence_job(&fx, "job-backup")).await;
    assert_eq!(status_of(&collected, "backup"), StepStatus::Failed);
    assert_eq!(status_of(&collected, "hook-after"), StepStatus::Success);
    assert_eq!(status_of(&collected, "hook-success"), StepStatus::Skipped);
    assert_eq!(status_of(&collected, "hook-error"), StepStatus::Success);
    assert_eq!(status_of(&collected, "hook-finally"), StepStatus::Success);
    let result = collected.result();
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(
        result.error.as_ref().map(|e| e.code.as_str()),
        Some("step_failed")
    );
}

/// FR-HK-05: a failed `on_success` hook fails the job, and `finally` still runs.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_05_a_failed_success_hook_fails_the_job_and_finally_runs() {
    let fx = HookFixture::new("hook-success-fails");
    fx.write_allow_list(&sequence_list(&fx));
    fx.script("h-success", "exit 2\n");
    let collected = run(&fx.jobs(), &sequence_job(&fx, "job-late")).await;
    assert_eq!(status_of(&collected, "backup"), StepStatus::Success);
    assert_eq!(status_of(&collected, "hook-success"), StepStatus::Failed);
    assert_eq!(status_of(&collected, "hook-finally"), StepStatus::Success);
    assert_eq!(collected.result().outcome, JobStatus::Failed as i32);
    assert_eq!(
        collected.result().error.as_ref().map(|e| e.code.as_str()),
        Some("hook_failed")
    );
}

/// FR-HK-05: a failed cleanup hook does not stop the next one of its phase.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_05_a_failed_cleanup_hook_does_not_stop_the_next_one() {
    let fx = HookFixture::new("hook-cleanup");
    fx.write_allow_list(&format!("{}{}", fx.hook("c-one", ""), fx.hook("c-two", "")));
    fx.script("c-one", "exit 1\n");
    let job = fx.job(
        "job-cleanup",
        vec![
            backup_step("repo-1", &["/srv"]),
            hook_step("hook-one", "c-one", HookPhase::After, vec![]),
            hook_step("hook-two", "c-two", HookPhase::After, vec![]),
        ],
    );
    let collected = run(&fx.jobs(), &job).await;
    assert_eq!(status_of(&collected, "hook-one"), StepStatus::Failed);
    assert_eq!(status_of(&collected, "hook-two"), StepStatus::Success);
    assert_eq!(fx.runs("c-two"), 1);
}

/// FR-HK-03: the output of a hook reaches the job log, bounded and scrubbed, and the exit code is in
/// the step result.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_03_the_output_and_the_exit_code_reach_the_job() {
    let fx = HookFixture::new("hook-output");
    fx.write_allow_list(&fx.hook("talk", ""));
    fx.script("talk", "out hello from the hook\nerr a warning\nexit 3\n");
    let job = fx.job(
        "job-talk",
        vec![hook_step("hook-talk", "talk", HookPhase::Before, vec![])],
    );
    let collected = run(&fx.jobs(), &job).await;
    let log = collected.log_text();
    assert!(log.contains("hello from the hook"), "{log}");
    assert!(log.contains("a warning"), "{log}");
    assert_eq!(collected.step("hook-talk").exit_code, 3);
    assert_eq!(status_of(&collected, "hook-talk"), StepStatus::Failed);
}

/// FR-HK-03: a hook cannot fill the job log.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_03_the_output_of_a_hook_is_bounded() {
    let fx = HookFixture::new("hook-flood");
    fx.write_allow_list(&fx.hook("flood", ""));
    let line = format!("out {}\n", "x".repeat(1000));
    fx.script("flood", &line.repeat(600));
    let job = fx.job(
        "job-flood",
        vec![hook_step("hook-flood", "flood", HookPhase::Before, vec![])],
    );
    let collected = run(&fx.jobs(), &job).await;
    let log = collected.log_text();
    assert!(log.contains("hook output truncated"), "{}", log.len());
    assert!(log.len() < 300 * 1024, "{}", log.len());
}

/// AC-HK-03: the agent rejects a job that refers to an unknown hook ID.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_03_an_unknown_hook_is_rejected() {
    let fx = HookFixture::new("hook-unknown");
    fx.write_allow_list(&fx.hook("known", ""));
    let job = fx.job(
        "job-unknown",
        vec![
            hook_step("hook-0", "known", HookPhase::Before, vec![]),
            hook_step("hook-1", "rm-everything", HookPhase::Before, vec![]),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    assert_eq!(
        rejection(&fx.jobs(), &job).await,
        Some(RejectReason::UnknownHook)
    );
    assert_eq!(fx.runs("known"), 0, "no hook of a rejected job runs");
    assert!(fx.restic_runs().is_empty());
}

/// AC-HK-03: a host without an allow-list file runs no hook.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_03_a_host_without_an_allow_list_rejects_every_hook() {
    let fx = HookFixture::new("hook-absent");
    let job = fx.job(
        "job-absent",
        vec![hook_step("hook-0", "any", HookPhase::Before, vec![])],
    );
    assert_eq!(
        rejection(&fx.jobs(), &job).await,
        Some(RejectReason::UnknownHook)
    );
    let (state, hooks) = fx.jobs().hook_report().await;
    assert_eq!(state, HookAllowListState::Absent);
    assert!(hooks.is_empty());
}

const PARAMETERS: &str = "
[[hook.parameter]]
name = 'database'
type = 'string'
max_length = 64
pattern = '[^\\x00]*'
required = false

[[hook.parameter]]
name = 'target'
type = 'string'
max_length = 64
pass_as = 'positional'
position = 1
required = true

[[hook.parameter]]
name = 'jobs'
type = 'integer'
minimum = 1
maximum = 8
required = false

[[hook.parameter]]
name = 'mode'
type = 'enum'
values = ['full', 'fast']
required = false

[[hook.parameter]]
name = 'verbose'
type = 'boolean'
required = false

[[hook.parameter]]
name = 'token'
type = 'secret_ref'
env_name = 'HOOK_TOKEN'
required = false
";

const INJECTIONS: [&str; 8] = [
    "; rm -rf /",
    "& del C:\\*",
    "$(id)",
    "`id`",
    "a b  c",
    "'quoted' \"double\"",
    "| cat /etc/passwd",
    "x\u{a0}y",
];

fn parameter_job(
    fx: &HookFixture,
    parameters: Vec<HookParameter>,
) -> restichub_proto::agent::v1::Job {
    fx.job(
        "job-parameters",
        vec![hook_step("hook-0", "tool", HookPhase::Before, parameters)],
    )
}

/// AC-HK-04: a hostile text goes to the hook as one literal argument.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_04_a_hostile_text_is_one_literal_argument() {
    let fx = HookFixture::new("hook-injection");
    fx.write_allow_list(&fx.hook("tool", PARAMETERS));
    let jobs = fx.jobs();
    for (index, value) in INJECTIONS.iter().enumerate() {
        let job = fx.job(
            &format!("job-injection-{index}"),
            vec![hook_step(
                "hook-0",
                "tool",
                HookPhase::Before,
                vec![
                    string_param("database", value),
                    string_param("target", value),
                ],
            )],
        );
        let collected = run(&jobs, &job).await;
        assert_eq!(
            status_of(&collected, "hook-0"),
            StepStatus::Success,
            "{value}"
        );
    }
    let arguments = fx.arguments("tool");
    assert_eq!(arguments.len(), INJECTIONS.len() * 2);
    for (index, value) in INJECTIONS.iter().enumerate() {
        assert_eq!(arguments[index * 2], format!("--database={value}"));
        assert_eq!(arguments[index * 2 + 1], *value);
    }
}

/// AC-HK-04: a value that looks like an option stays one argument for a flag, and a positional
/// value cannot start with a dash.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_04_a_value_cannot_become_an_option() {
    let fx = HookFixture::new("hook-dash");
    fx.write_allow_list(&fx.hook("tool", PARAMETERS));
    let jobs = fx.jobs();
    let flag_job = parameter_job(
        &fx,
        vec![
            string_param("database", "--delete-everything"),
            string_param("target", "app"),
        ],
    );
    let collected = run(&jobs, &flag_job).await;
    assert_eq!(status_of(&collected, "hook-0"), StepStatus::Success);
    assert_eq!(
        fx.arguments("tool"),
        ["--database=--delete-everything", "app"]
    );
    for value in ["--delete-everything", "-x", "--"] {
        let job = fx.job(
            "job-dash",
            vec![hook_step(
                "hook-0",
                "tool",
                HookPhase::Before,
                vec![string_param("target", value)],
            )],
        );
        assert_eq!(
            rejection(&jobs, &job).await,
            Some(RejectReason::InvalidHookParameters),
            "{value}"
        );
    }
    assert_eq!(fx.runs("tool"), 1);
}

/// AC-HK-04: a value that disagrees with the schema is rejected before the hook starts.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_04_a_value_outside_the_schema_is_rejected() {
    let fx = HookFixture::new("hook-schema");
    fx.write_allow_list(&fx.hook("tool", PARAMETERS));
    let jobs = fx.jobs();
    let target = || string_param("target", "app");
    let cases: Vec<(&str, Vec<HookParameter>)> = vec![
        (
            "integer above the range",
            vec![target(), integer_param("jobs", 9)],
        ),
        (
            "integer below the range",
            vec![target(), integer_param("jobs", 0)],
        ),
        ("integer as text", vec![target(), string_param("jobs", "4")]),
        (
            "enum outside the list",
            vec![target(), string_param("mode", "slow")],
        ),
        (
            "enum with an injection",
            vec![target(), string_param("mode", "full; id")],
        ),
        (
            "boolean as text",
            vec![target(), string_param("verbose", "true")],
        ),
        (
            "unknown parameter",
            vec![target(), string_param("extra", "x")],
        ),
        ("duplicate parameter", vec![target(), target()]),
        (
            "missing required parameter",
            vec![string_param("database", "x")],
        ),
        ("too long", vec![string_param("target", &"x".repeat(65))]),
        ("secret as text", vec![target(), string_param("token", "x")]),
        ("text as secret", vec![secret_param("target", "x")]),
        (
            "legacy value field",
            vec![
                target(),
                HookParameter {
                    name: "database".to_owned(),
                    value: "x".to_owned(),
                    typed: None,
                },
            ],
        ),
        (
            "no value",
            vec![
                target(),
                HookParameter {
                    name: "database".to_owned(),
                    ..HookParameter::default()
                },
            ],
        ),
    ];
    for (name, parameters) in cases {
        let job = parameter_job(&fx, parameters);
        assert_eq!(
            rejection(&jobs, &job).await,
            Some(RejectReason::InvalidHookParameters),
            "{name}"
        );
    }
    assert_eq!(fx.runs("tool"), 0, "no hook of a rejected job runs");
    let valid = parameter_job(
        &fx,
        vec![
            target(),
            integer_param("jobs", 8),
            string_param("mode", "fast"),
            bool_param("verbose", true),
            bool_param("verbose2", false),
        ],
    );
    assert_eq!(
        rejection(&jobs, &valid).await,
        Some(RejectReason::InvalidHookParameters),
        "an unknown name stays refused"
    );
    let valid = parameter_job(
        &fx,
        vec![
            target(),
            integer_param("jobs", 8),
            string_param("mode", "fast"),
            bool_param("verbose", true),
        ],
    );
    let collected = run(&jobs, &valid).await;
    assert_eq!(status_of(&collected, "hook-0"), StepStatus::Success);
    assert_eq!(
        fx.arguments("tool"),
        ["--jobs=8", "--mode=fast", "--verbose", "app"]
    );
}

/// AC-HK-04: a pattern must match the whole value.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_04_a_pattern_matches_the_whole_value() {
    let fx = HookFixture::new("hook-pattern");
    fx.write_allow_list(&fx.hook(
        "tool",
        "[[hook.parameter]]\nname = 'database'\ntype = 'string'\npattern = '[a-z_][a-z0-9_]*'\n",
    ));
    let jobs = fx.jobs();
    for value in ["app_db", "x1"] {
        let job = parameter_job(&fx, vec![string_param("database", value)]);
        assert_eq!(
            status_of(&run(&jobs, &job).await, "hook-0"),
            StepStatus::Success
        );
    }
    for value in ["app; rm -rf /", "App", "1x", "", "a b", "$(id)"] {
        let job = parameter_job(&fx, vec![string_param("database", value)]);
        assert_eq!(
            rejection(&jobs, &job).await,
            Some(RejectReason::InvalidHookParameters),
            "{value:?}"
        );
    }
    // The job validation refuses a control character before the schema applies.
    let job = parameter_job(&fx, vec![string_param("database", "a\n")]);
    assert_eq!(rejection(&jobs, &job).await, Some(RejectReason::Malformed));
}

/// FR-HK-03: the hook starts with a cleared environment, the declared variables and its directory.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_03_the_hook_gets_only_the_declared_environment() {
    let fx = HookFixture::new("hook-env");
    let work = fx
        .work
        .canonicalize()
        .expect("cannot resolve the work directory");
    fx.write_allow_list(&fx.hook(
        "tool",
        &format!(
            "working_dir = '{}'\n[hook.env]\nFIXED = 'one'\n{PARAMETERS}",
            work.display()
        ),
    ));
    let job = parameter_job(
        &fx,
        vec![string_param("target", "app"), secret_param("token", CANARY)],
    );
    let collected = run(&fx.jobs(), &job).await;
    assert_eq!(status_of(&collected, "hook-0"), StepStatus::Success);
    let names: Vec<String> = fx
        .environment("tool")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["FIXED", "HOOK_TOKEN", "PATH"]);
    assert!(
        fx.record("tool")
            .contains(&format!("cwd:{}", work.display()))
    );
}

/// FR-HK-03, ADR-0017: a secret parameter reaches the hook only through its environment variable.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_03_a_secret_parameter_is_in_the_environment_only() {
    let fx = HookFixture::new("hook-secret");
    fx.write_allow_list(&fx.hook("tool", PARAMETERS));
    fx.script("tool", "env HOOK_TOKEN\nerr the token is HOOK_TOKEN\n");
    let job = parameter_job(
        &fx,
        vec![string_param("target", "app"), secret_param("token", CANARY)],
    );
    let jobs = fx.jobs();
    let collected = run(&jobs, &job).await;
    assert_eq!(status_of(&collected, "hook-0"), StepStatus::Success);
    assert_eq!(fx.arguments("tool"), ["app"]);
    assert!(
        fx.environment("tool")
            .contains(&("HOOK_TOKEN".to_owned(), CANARY.to_owned())),
        "the hook reads the secret from its environment"
    );
    let log = collected.log_text();
    assert!(!log.contains(CANARY), "the log must not hold the secret");
    assert!(log.contains("***"), "the output masks the secret: {log}");
    for message in &collected.messages {
        assert!(!format!("{message:?}").contains(CANARY));
    }
    assert!(!format!("{:?}", job.steps).contains(CANARY));
    for entry in std::fs::read_dir(&fx.dir).expect("cannot read the agent directory") {
        let path = entry.expect("a directory entry").path();
        if path.is_file() {
            let bytes = std::fs::read(&path).expect("cannot read the file");
            assert!(
                !bytes
                    .windows(CANARY.len())
                    .any(|window| window == CANARY.as_bytes()),
                "{} holds the secret",
                path.display()
            );
        }
    }
}

/// FR-HK-02: the timeout of a hook stops it in two stages and kills the tree (FR-AG-10).
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_02_a_timeout_kills_the_whole_tree() {
    let fx = HookFixture::new("hook-timeout");
    fx.write_allow_list(&format!(
        "{}{}",
        fx.hook("slow", "timeout_seconds = 1"),
        fx.hook("h-error", "")
    ));
    fx.script("slow", "child\nignore-interrupt\nsleep 60000\n");
    let job = fx.job(
        "job-timeout",
        vec![
            hook_step("hook-slow", "slow", HookPhase::Before, vec![]),
            backup_step("repo-1", &["/srv"]),
            hook_step("hook-error", "h-error", HookPhase::OnError, vec![]),
        ],
    );
    let jobs = fx
        .jobs()
        .with_stop_grace(std::time::Duration::from_millis(300));
    let collected = run(&jobs, &job).await;
    assert_eq!(status_of(&collected, "hook-slow"), StepStatus::Failed);
    assert_eq!(
        collected
            .step("hook-slow")
            .error
            .as_ref()
            .map(|e| e.code.as_str()),
        Some("step_timeout")
    );
    assert_eq!(status_of(&collected, "hook-error"), StepStatus::Success);
    assert!(fx.restic_runs().is_empty());
    let pid = std::fs::read_to_string(format!("{}.child", fx.control("slow").display()))
        .expect("the hook started a grandchild");
    let proc = std::path::PathBuf::from(format!("/proc/{}", pid.trim()));
    let mut alive = true;
    for _ in 0..50 {
        // A zombie keeps its entry until the parent reaps it; its state tells that it is dead.
        alive = std::fs::read_to_string(proc.join("stat")).is_ok_and(|stat| !stat.contains(") Z"));
        if !alive {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!alive, "the grandchild must die with the tree");
}

/// FR-AG-10: a cancel stops the running hook, and only the `finally` hooks run after it.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_05_a_cancel_stops_the_hook_and_runs_the_finally_hooks() {
    let fx = HookFixture::new("hook-cancel");
    fx.write_allow_list(&sequence_list(&fx));
    fx.script("h-before", "sleep 60000\n");
    let jobs = fx
        .jobs()
        .with_stop_grace(std::time::Duration::from_millis(300));
    let job = sequence_job(&fx, "job-cancel");
    let (tx, mut rx) = mpsc::channel(256);
    jobs.attach(&tx);
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    for _ in 0..200 {
        if fx.runs("h-before") > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    jobs.cancel("job-cancel");
    let collected = collect(&mut rx).await;
    assert_eq!(collected.result().outcome, JobStatus::Cancelled as i32);
    assert_eq!(status_of(&collected, "hook-before"), StepStatus::Cancelled);
    assert_eq!(status_of(&collected, "hook-finally"), StepStatus::Success);
    assert_eq!(fx.runs("h-error"), 0);
    assert_eq!(fx.runs("h-success"), 0);
    assert!(fx.restic_runs().is_empty());
}

/// AC-HK-01: a writable allow-list file stops every hook, and the agent reports the state.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_01_a_writable_allow_list_runs_no_hook() {
    let fx = HookFixture::new("hook-writable");
    fx.write_allow_list(&fx.hook("tool", ""));
    let job = fx.job(
        "job-writable",
        vec![hook_step("hook-0", "tool", HookPhase::Before, vec![])],
    );
    let jobs = fx.jobs();
    assert_eq!(
        status_of(&run(&jobs, &job).await, "hook-0"),
        StepStatus::Success,
        "the same file runs while the mode is safe"
    );
    for mode in [0o666, 0o664, 0o646] {
        set_mode(&fx.allow_list(), mode);
        let retry = fx.job(
            &format!("job-writable-{mode}"),
            vec![hook_step("hook-0", "tool", HookPhase::Before, vec![])],
        );
        assert_eq!(
            rejection(&jobs, &retry).await,
            Some(RejectReason::AllowListInsecure),
            "mode {mode:o}"
        );
        let (state, hooks) = jobs.hook_report().await;
        assert_eq!(state, HookAllowListState::Insecure);
        assert!(hooks.is_empty());
    }
    assert_eq!(
        fx.runs("tool"),
        1,
        "no hook ran after the file became writable"
    );
}

/// AC-HK-01: a directory that others can write makes the file untrusted.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_01_a_writable_directory_runs_no_hook() {
    let fx = HookFixture::new("hook-dir");
    fx.write_allow_list(&fx.hook("tool", ""));
    set_mode(&fx.work, 0o777);
    let job = fx.job(
        "job-dir",
        vec![hook_step("hook-0", "tool", HookPhase::Before, vec![])],
    );
    assert_eq!(
        rejection(&fx.jobs(), &job).await,
        Some(RejectReason::AllowListInsecure)
    );
    set_mode(&fx.work, 0o755);
}

/// AC-HK-01: the host trust accepts root and Administrators only.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_01_the_host_trust_refuses_a_file_of_a_user_account() {
    use std::os::unix::fs::MetadataExt as _;
    let fx = HookFixture::new("hook-host");
    fx.write_allow_list(&fx.hook("tool", ""));
    let owner = std::fs::metadata(fx.allow_list())
        .expect("cannot read the file")
        .uid();
    if owner == 0 {
        return;
    }
    let job = fx.job(
        "job-host",
        vec![hook_step("hook-0", "tool", HookPhase::Before, vec![])],
    );
    assert_eq!(
        rejection(&fx.jobs_with(Trust::Host), &job).await,
        Some(RejectReason::AllowListInsecure)
    );
    assert_eq!(fx.runs("tool"), 0);
}

/// AC-HK-01: an executable that others can write is refused, and the agent does not list it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_01_a_writable_executable_is_refused() {
    let fx = HookFixture::new("hook-exe");
    let copy = fx.work.join("copy-of-hook");
    std::fs::copy(&fx.executable, &copy).expect("cannot copy the hook");
    set_mode(&copy, 0o755);
    fx.write_allow_list(&fx.hook_with(&copy, "tool", ""));
    let job = fx.job(
        "job-exe",
        vec![hook_step("hook-0", "tool", HookPhase::Before, vec![])],
    );
    let jobs = fx.jobs();
    assert_eq!(
        status_of(&run(&jobs, &job).await, "hook-0"),
        StepStatus::Success
    );
    assert_eq!(jobs.hook_report().await.1.len(), 1);

    set_mode(&copy, 0o777);
    let retry = fx.job(
        "job-exe-2",
        vec![hook_step("hook-0", "tool", HookPhase::Before, vec![])],
    );
    assert_eq!(
        rejection(&jobs, &retry).await,
        Some(RejectReason::AllowListInsecure)
    );
    let (state, hooks) = jobs.hook_report().await;
    assert_eq!(state, HookAllowListState::Loaded);
    assert!(
        hooks.is_empty(),
        "the agent does not offer a hook it cannot run"
    );
    assert_eq!(fx.runs("tool"), 1);
}

/// AC-HK-01: a file that changes after the admission of the job stops the hook at its start.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_01_the_file_is_checked_again_when_the_hook_starts() {
    let fx = HookFixture::new("hook-recheck");
    fx.write_allow_list(&sequence_list(&fx));
    fx.script("h-before", "sleep 700\n");
    let jobs = fx.jobs();
    let job = sequence_job(&fx, "job-recheck");
    let (tx, mut rx) = mpsc::channel(256);
    jobs.attach(&tx);
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    for _ in 0..200 {
        if fx.runs("h-before") > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    // The file becomes writable while the first hook runs: the next hook must not start.
    set_mode(&fx.allow_list(), 0o666);
    let collected = collect(&mut rx).await;
    assert_eq!(fx.runs("h-after"), 0);
    assert_eq!(fx.runs("h-finally"), 0);
    assert_eq!(collected.result().outcome, JobStatus::Failed as i32);
}

/// FR-HK-04: the agent reports the identifiers and the schemas, and no path.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_04_the_report_holds_the_schema_and_no_path() {
    let fx = HookFixture::new("hook-report");
    fx.write_allow_list(&fx.hook(
        "tool",
        &format!("phases = ['before', 'finally']\n[hook.env]\nSECRETISH = 'fixed'\n{PARAMETERS}"),
    ));
    let (state, hooks) = fx.jobs().hook_report().await;
    assert_eq!(state, HookAllowListState::Loaded);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].hook_id, "tool");
    assert_eq!(hooks[0].parameters.len(), 6);
    assert_eq!(hooks[0].phases.len(), 2);
    let text = format!("{hooks:?}");
    assert!(!text.contains("fake-hook"), "{text}");
    assert!(!text.contains("--control"), "{text}");
    assert!(!text.contains("SECRETISH"), "{text}");
}

/// FR-HK-05: a hook runs in the phases of its entry only.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_05_a_hook_runs_in_its_phases_only() {
    let fx = HookFixture::new("hook-phases");
    fx.write_allow_list(&fx.hook("tool", "phases = ['before']"));
    let jobs = fx.jobs();
    let wrong = fx.job(
        "job-phase",
        vec![hook_step("hook-0", "tool", HookPhase::Finally, vec![])],
    );
    assert_eq!(
        rejection(&jobs, &wrong).await,
        Some(RejectReason::InvalidHookParameters)
    );
    let unspecified = fx.job(
        "job-phase-0",
        vec![hook_step("hook-0", "tool", HookPhase::Unspecified, vec![])],
    );
    assert_eq!(
        rejection(&jobs, &unspecified).await,
        Some(RejectReason::InvalidHookParameters)
    );
}

/// AC-HK-02: 100 jobs of different types do not change the allow-list.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_02_a_hundred_jobs_leave_the_allow_list_unchanged() {
    let fx = HookFixture::new("hook-checksum");
    fx.write_allow_list(&sequence_list(&fx));
    let before = sha256_hex(&fx.allow_list());
    let jobs = fx.jobs();
    for index in 0..100 {
        let steps = match index % 4 {
            0 => vec![backup_step("repo-1", &["/srv"])],
            1 => vec![check_step("repo-1")],
            2 => vec![
                hook_step("hook-0", "h-before", HookPhase::Before, vec![]),
                hook_step("hook-1", "h-finally", HookPhase::Finally, vec![]),
            ],
            _ => sequence_job(&fx, "unused").steps,
        };
        let collected = run(&jobs, &fx.job(&format!("job-{index}"), steps)).await;
        assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
    }
    assert_eq!(sha256_hex(&fx.allow_list()), before);
}
