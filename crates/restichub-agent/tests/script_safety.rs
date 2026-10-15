//! The safety rules of the script steps: the search path, the journal and the cleanup phases
//! (AC-HK-01, FR-HK-08, ADR-0037).

#![allow(clippy::expect_used)]
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt as _;

use common::scripts::*;
use common::*;
use restichub_proto::agent::v1::{HookPhase, StepStatus};

fn directory(host: &Host, name: &str, mode: u32) -> std::path::PathBuf {
    let dir = host.fx.work.join(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).expect("mode");
    dir
}

/// AC-HK-01: a search path directory that another account can write never reaches a script, and
/// the step log names the dropped directory once for the job.
#[tokio::test(flavor = "multi_thread")]
async fn ac_hk_01_a_user_writable_search_path_directory_is_not_in_the_script_environment() {
    let host = Host::new("script-path");
    host.enable();
    let good = directory(&host, "bin-good", 0o755);
    let bad = directory(&host, "bin-bad", 0o777);
    let path = std::env::join_paths([good.as_path(), bad.as_path()]).expect("join");
    let record = |name: &str| format!("echo \"$PATH\" > '{}'\n", host.marks().join(name).display());
    let job = host.job(
        "job-path",
        vec![
            simple("script-0", HookPhase::Before, &record("path-0"), vec![]),
            simple("script-1", HookPhase::Before, &record("path-1"), vec![]),
            backup_step("repo-1", &["/srv"]),
        ],
    );
    let collected = run(&host.jobs_with_path(path), &job).await;
    for step_id in ["script-0", "script-1"] {
        assert_eq!(status_of(&collected, step_id), StepStatus::Success);
    }
    for name in ["path-0", "path-1"] {
        let seen = host.mark(name);
        let entries: Vec<_> = std::env::split_paths(seen.trim_end()).collect();
        assert_eq!(entries, vec![good.clone()], "{name}");
    }
    let log = collected.log_text();
    let bad = bad.display().to_string();
    assert_eq!(log.matches(&bad).count(), 1, "{log}");
}
