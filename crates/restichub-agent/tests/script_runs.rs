//! The script steps against real interpreters, without a control plane (FR-HK-08 to FR-HK-11,
//! AC-HK-07 to AC-HK-10, AC-HK-13, ADR-0037).
//!
//! The tests drive [`Jobs::handle`] directly. The setting file belongs to the current user, so they
//! use the `CurrentUser` trust; the host trust has its own unit tests. A script writes its marks to a
//! directory of the test, so the tests read what the interpreter really did.

#![allow(clippy::expect_used)]

mod common;

use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

use common::scripts::*;
use common::*;
#[cfg(unix)]
use restichub_proto::agent::v1::JobStatus;
use restichub_proto::agent::v1::{
    HookPhase, RejectReason, ScriptFailurePolicy, ScriptInterpreter, StepStatus,
};
#[cfg(unix)]
use tokio::sync::mpsc;

const CANARY: &str = "CANARY-script-secret-4c1d";

/// AC-HK-07: without the local setting the agent rejects the job and runs no script.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_07_a_host_without_the_trust_mode_rejects_the_job() {
    let host = Host::new("script-off");
    let body = mark_body(&host, "ran");
    let job = host.job(
        "job-off",
        vec![
            simple("script-0", HookPhase::Before, &body, vec![]),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    assert_eq!(
        rejection(&host.jobs(), &job).await,
        Some(RejectReason::ServerScriptsDisabled)
    );
    assert_eq!(host.mark("order"), "");
    assert!(host.fx.restic_runs().is_empty());
}

/// AC-HK-08: a setting that other accounts can write turns the mode off.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_08_a_setting_that_others_can_write_runs_no_script() {
    let host = Host::new("script-writable");
    host.write_setting("server_scripts = enabled\n", 0o666);
    let body = mark_body(&host, "ran");
    let job = host.job(
        "job-writable",
        vec![simple("script-0", HookPhase::Before, &body, vec![])],
    );
    assert_eq!(
        rejection(&host.jobs(), &job).await,
        Some(RejectReason::ServerScriptsDisabled)
    );
    assert_eq!(host.mark("order"), "");
}

/// FR-HK-09: a broken variable name is rejected before anything runs.
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_09_a_reserved_variable_name_is_rejected() {
    let host = Host::new("script-name");
    host.enable();
    let job = host.job(
        "job-name",
        vec![simple(
            "script-0",
            HookPhase::Before,
            "echo",
            vec![variable("LD_PRELOAD", "/tmp/x.so")],
        )],
    );
    assert_eq!(
        rejection(&host.jobs(), &job).await,
        Some(RejectReason::InvalidScript)
    );
}

/// FR-HK-09, AC-HK-09: the script reads its variables; the secret is in the environment only.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_09_the_secret_reaches_the_script_in_the_environment_only() {
    let host = Host::new("script-secret");
    host.enable();
    let marks = host.marks();
    let body = format!(
        "printf '%s' \"$PGPASSWORD\" > '{marks}/seen'\n\
         echo \"host=$PGHOST\"\n\
         echo \"leak=$PGPASSWORD\"\n\
         echo \"args=$*\"\n\
         tr '\\0' ' ' < /proc/$$/cmdline\n\
         echo\n\
         echo \"file=$0\"\n\
         cat \"$0\" > '{marks}/body'\n",
        marks = marks.display()
    );
    let job = host.job(
        "job-secret",
        vec![
            simple(
                "script-0",
                HookPhase::Before,
                &body,
                vec![variable("PGHOST", "db1"), secret("PGPASSWORD", CANARY)],
            ),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Success);
    assert_eq!(host.mark("seen"), CANARY, "the script reads the secret");
    assert_eq!(host.mark("body"), body, "the file holds the body");
    let log = collected.log_text();
    assert!(log.contains("host=db1"), "{log}");
    assert!(!log.contains(CANARY), "the log holds the secret: {log}");
    assert!(log.contains("leak=***"), "{log}");
    // The body file is gone after the run, and so is its directory.
    let file = log
        .lines()
        .find_map(|line| line.strip_prefix("file="))
        .expect("the script printed its file");
    assert!(!Path::new(file.trim()).exists(), "{file}");
    assert!(host.leftovers().is_empty(), "{:?}", host.leftovers());
    // No file of the agent holds the secret: the journal, the cache and the configuration.
    assert!(!contains_bytes(&host.fx.dir, CANARY));
    assert!(!contains_bytes(&host.cache(), CANARY));
    // The control plane receives no secret in a result message either.
    let text = format!("{:?}", collected.messages);
    assert!(!text.contains(CANARY));
}

/// FR-HK-05: the scripts follow the phase order and each one is its own step.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_11_the_scripts_run_in_the_phase_order_as_separate_steps() {
    let host = Host::new("script-order");
    host.enable();
    let job = host.job(
        "job-order",
        vec![
            simple(
                "script-4",
                HookPhase::Finally,
                &mark_body(&host, "finally"),
                vec![],
            ),
            simple(
                "script-1",
                HookPhase::Before,
                &mark_body(&host, "before-1"),
                vec![],
            ),
            simple(
                "script-2",
                HookPhase::Before,
                &mark_body(&host, "before-2"),
                vec![],
            ),
            backup_step("repo-1", &["/srv"]),
            simple(
                "script-3",
                HookPhase::After,
                &mark_body(&host, "after"),
                vec![],
            ),
            simple(
                "script-5",
                HookPhase::OnSuccess,
                &mark_body(&host, "success"),
                vec![],
            ),
            simple(
                "script-6",
                HookPhase::OnError,
                &mark_body(&host, "error"),
                vec![],
            ),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(
        host.mark("order"),
        "before-1\nbefore-2\nafter\nsuccess\nfinally\n"
    );
    assert_eq!(status_of(&collected, "script-6"), StepStatus::Skipped);
    assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
    // Each step has its own start and end time and its own log lines (FR-HK-11).
    let first = collected.step("script-1");
    assert!(first.started_at.is_some() && first.finished_at.is_some());
    assert_eq!(host.fx.restic_runs().len(), 1);
}

/// AC-HK-13: a failed `before` script with the policy `abort_backup` stops the backup; the
/// `on_error` and `finally` scripts run.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_13_a_failed_before_script_aborts_the_backup() {
    let host = Host::new("script-abort");
    host.enable();
    let job = host.job(
        "job-abort",
        vec![
            simple(
                "script-0",
                HookPhase::Before,
                "echo the dump failed >&2\nexit 7\n",
                vec![],
            ),
            backup_step("repo-1", &["/srv"]),
            simple(
                "script-1",
                HookPhase::After,
                &mark_body(&host, "after"),
                vec![],
            ),
            simple(
                "script-2",
                HookPhase::OnSuccess,
                &mark_body(&host, "success"),
                vec![],
            ),
            simple(
                "script-3",
                HookPhase::OnError,
                &mark_body(&host, "error"),
                vec![],
            ),
            simple(
                "script-4",
                HookPhase::Finally,
                &mark_body(&host, "finally"),
                vec![],
            ),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert!(host.fx.restic_runs().is_empty(), "the backup must not run");
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Failed);
    assert_eq!(collected.step("script-0").exit_code, 7);
    assert_eq!(status_of(&collected, "backup"), StepStatus::Skipped);
    assert_eq!(host.mark("order"), "error\nfinally\n");
    let result = collected.result();
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    let error = result.error.as_ref().expect("a typed reason");
    assert_eq!(error.code, "hook_failed");
    assert!(
        error
            .params
            .iter()
            .any(|pair| pair.key == "hook" && pair.value == "name-script-0")
    );
    assert!(collected.log_text().contains("the dump failed"));
}

/// AC-HK-13: with the policy `continue` the backup runs and the job ends with warnings.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_13_a_failed_script_with_the_continue_policy_does_not_stop_the_backup() {
    let host = Host::new("script-continue");
    host.enable();
    let job = host.job(
        "job-continue",
        vec![
            script_step(
                "script-0",
                HookPhase::Before,
                ScriptFailurePolicy::Continue,
                "exit 3\n",
                vec![],
            ),
            backup_step("repo-1", &["/srv"]),
            simple(
                "script-1",
                HookPhase::OnSuccess,
                &mark_body(&host, "success"),
                vec![],
            ),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Failed);
    assert_eq!(status_of(&collected, "backup"), StepStatus::Success);
    assert_eq!(host.fx.restic_runs().len(), 1);
    assert_eq!(host.mark("order"), "success\n");
    assert_eq!(
        collected.result().outcome,
        JobStatus::SucceededWithWarnings as i32
    );
}

/// FR-HK-09: the output of a script is bounded.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_09_the_output_of_a_script_is_bounded() {
    let host = Host::new("script-flood");
    host.enable();
    let job = host.job(
        "job-flood",
        vec![simple(
            "script-0",
            HookPhase::Before,
            "yes xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx | head -n 100000\n",
            vec![],
        )],
    );
    let collected = run(&host.jobs(), &job).await;
    let log = collected.log_text();
    assert!(log.contains("script output truncated"), "{}", log.len());
    assert!(log.len() < 300 * 1024, "{}", log.len());
}

/// AC-HK-10: the timeout stops the script and its child processes.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_10_a_timeout_kills_the_whole_tree() {
    let host = Host::new("script-timeout");
    host.enable();
    let pid_file = host.marks().join("child.pid");
    let body = format!(
        "trap '' INT TERM\nsleep 300 &\necho $! > '{}'\nwait\n",
        pid_file.display()
    );
    let mut slow = simple("script-0", HookPhase::Before, &body, vec![]);
    slow.timeout_seconds = 1;
    let job = host.job(
        "job-timeout",
        vec![
            slow,
            backup_step("repo-1", &["/srv"]),
            simple(
                "script-1",
                HookPhase::OnError,
                &mark_body(&host, "error"),
                vec![],
            ),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Failed);
    assert_eq!(
        collected
            .step("script-0")
            .error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("step_timeout")
    );
    assert_eq!(host.mark("order"), "error\n");
    let pid = std::fs::read_to_string(&pid_file).expect("the script started a child");
    let proc = PathBuf::from(format!("/proc/{}", pid.trim()));
    let mut alive = true;
    for _ in 0..50 {
        alive = std::fs::read_to_string(proc.join("stat")).is_ok_and(|stat| !stat.contains(") Z"));
        if !alive {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!alive, "the child must die with the tree");
    assert!(host.leftovers().is_empty());
}

/// AC-HK-10: a cancel stops the running script; the `finally` scripts still run.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_10_a_cancel_stops_the_script_and_runs_the_finally_script() {
    let host = Host::new("script-cancel");
    host.enable();
    let started = host.marks().join("started");
    let body = format!("touch '{}'\nsleep 300\n", started.display());
    let job = host.job(
        "job-cancel",
        vec![
            simple("script-0", HookPhase::Before, &body, vec![]),
            backup_step("repo-1", &["/srv"]),
            simple(
                "script-1",
                HookPhase::Finally,
                &mark_body(&host, "finally"),
                vec![],
            ),
        ],
    );
    let jobs = host.jobs();
    let (tx, mut rx) = mpsc::channel(256);
    jobs.attach(&tx);
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    for _ in 0..200 {
        if started.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    jobs.cancel("job-cancel");
    let collected = collect(&mut rx).await;
    assert_eq!(collected.result().outcome, JobStatus::Cancelled as i32);
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Cancelled);
    assert_eq!(host.mark("order"), "finally\n");
    assert!(host.fx.restic_runs().is_empty());
    assert!(host.leftovers().is_empty());
}

/// FR-HK-08: the agent reads the setting at every job, so a change applies at the next job.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_08_a_disabled_setting_stops_the_next_job() {
    let host = Host::new("script-toggle");
    host.enable();
    let jobs = host.jobs();
    let job = |id: &str| {
        host.job(
            id,
            vec![simple("script-0", HookPhase::Before, "true\n", vec![])],
        )
    };
    let first = run(&jobs, &job("job-on")).await;
    assert_eq!(status_of(&first, "script-0"), StepStatus::Success);
    host.write_setting("server_scripts = disabled\n", 0o644);
    assert_eq!(
        rejection(&jobs, &job("job-off")).await,
        Some(RejectReason::ServerScriptsDisabled)
    );
}

/// FR-HK-08: the agent reports the mode and the interpreters in the hello.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn fr_hk_08_the_report_lists_the_interpreters_only_when_the_mode_is_on() {
    use restichub_proto::agent::v1::ServerScriptsState;
    let host = Host::new("script-report");
    let off = host.jobs().script_report().await;
    assert_eq!(off.0, ServerScriptsState::Disabled);
    assert!(off.1.is_empty());
    host.enable();
    let on = host.jobs().script_report().await;
    assert_eq!(on.0, ServerScriptsState::Enabled);
    assert!(on.1.contains(&ScriptInterpreter::Bash));
    assert!(!on.1.contains(&ScriptInterpreter::Powershell));
}

/// FR-HK-09 on Windows: a PowerShell script reads its variables, the secret stays out of the log,
/// and the script file goes away (AC-HK-09).
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_09_a_powershell_script_reads_the_secret_from_the_environment() {
    let host = Host::new("script-powershell");
    host.enable();
    let body = "Set-Content -Path (Join-Path $env:MARKS 'seen') -Value $env:PGPASSWORD -NoNewline\r\n\
                Write-Output \"host=$($env:PGHOST)\"\r\n\
                Write-Output \"leak=$($env:PGPASSWORD)\"\r\n\
                Write-Output \"file=$PSCommandPath\"\r\n";
    let job = host.job(
        "job-powershell",
        vec![
            interpreted(
                "script-0",
                ScriptInterpreter::Powershell,
                HookPhase::Before,
                ScriptFailurePolicy::AbortBackup,
                body,
                vec![
                    variable("MARKS", &host.marks().display().to_string()),
                    variable("PGHOST", "db1"),
                    secret("PGPASSWORD", CANARY),
                ],
            ),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Success);
    assert_eq!(host.mark("seen"), CANARY);
    let log = collected.log_text();
    assert!(log.contains("host=db1"), "{log}");
    assert!(!log.contains(CANARY), "{log}");
    assert!(log.contains("leak=***"), "{log}");
    let file = log
        .lines()
        .find_map(|line| line.strip_prefix("file="))
        .expect("the script printed its file");
    assert!(!Path::new(file.trim()).exists(), "{file}");
    assert!(host.leftovers().is_empty(), "{:?}", host.leftovers());
    assert!(!contains_bytes(&host.fx.dir, CANARY));
}

/// FR-HK-09 on Windows: a cmd script follows the failure policy.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_13_a_failed_cmd_script_aborts_the_backup() {
    let host = Host::new("script-cmd");
    host.enable();
    let job = host.job(
        "job-cmd",
        vec![
            interpreted(
                "script-0",
                ScriptInterpreter::Cmd,
                HookPhase::Before,
                ScriptFailurePolicy::AbortBackup,
                "echo the dump failed 1>&2\nexit /b 7\n",
                vec![],
            ),
            backup_step("repo-1", &["/srv"]),
            interpreted(
                "script-1",
                ScriptInterpreter::Cmd,
                HookPhase::Finally,
                ScriptFailurePolicy::Continue,
                "echo %MARK% > \"%MARKS%\\finally\"\n",
                vec![
                    variable("MARKS", &host.marks().display().to_string()),
                    variable("MARK", "done"),
                ],
            ),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert!(host.fx.restic_runs().is_empty());
    assert_eq!(status_of(&collected, "script-0"), StepStatus::Failed);
    assert_eq!(collected.step("script-0").exit_code, 7);
    assert_eq!(status_of(&collected, "script-1"), StepStatus::Success);
    assert!(host.mark("finally").contains("done"));
    assert!(host.leftovers().is_empty());
}

/// Runs a script on Windows that calls `whoami` by name and records its `PATH`.
#[cfg(windows)]
async fn run_whoami_script(name: &str, interpreter: ScriptInterpreter, body: &str) -> Host {
    let host = Host::new(name);
    host.enable();
    let job = host.job(
        "job-whoami",
        vec![
            interpreted(
                "script-0",
                interpreter,
                HookPhase::Before,
                ScriptFailurePolicy::AbortBackup,
                body,
                vec![variable("MARKS", &host.marks().display().to_string())],
            ),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    let collected = run(&host.jobs(), &job).await;
    assert_eq!(
        status_of(&collected, "script-0"),
        StepStatus::Success,
        "{}",
        collected.log_text()
    );
    let path = host.mark("path");
    assert!(!path.contains(r"\\?\"), "{path}");
    assert!(path.to_lowercase().contains("system32"), "{path}");
    assert!(!host.mark("who").trim().is_empty());
    host
}

/// ADR-0037, AC-HK-08 on Windows: a cmd script finds `whoami` by name, and its `PATH` holds no
/// verbatim `\\?\` entry.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_08_a_cmd_script_runs_a_system_program_by_name() {
    let body =
        "whoami > \"%MARKS%\\who\"\nif errorlevel 1 exit /b 1\n>\"%MARKS%\\path\" echo %PATH%\n";
    run_whoami_script("script-cmd-path", ScriptInterpreter::Cmd, body).await;
}

/// ADR-0037, AC-HK-08 on Windows: a PowerShell script finds `whoami` by name, and its `PATH` holds
/// no verbatim `\\?\` entry.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_08_a_powershell_script_runs_a_system_program_by_name() {
    let body = "whoami | Set-Content -Path (Join-Path $env:MARKS 'who')\r\n\
                if ($LASTEXITCODE -ne 0) { exit 1 }\r\n\
                Set-Content -Path (Join-Path $env:MARKS 'path') -Value $env:PATH\r\n";
    run_whoami_script("script-ps-path", ScriptInterpreter::Powershell, body).await;
}
