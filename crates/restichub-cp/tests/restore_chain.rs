//! A restore through the control plane, the signed job and the real agent runner with the real
//! restic (FR-RS-03, AC-RS-03, AC-PR-03, ADR-0003, ADR-0017).
//!
//! The backup comes from the restic command line, as an agent A would write it. The control plane
//! creates the restore job for agent B, which has its own directory and journal, and the real job
//! runner of the agent restores the snapshot. The test compares the SHA-256 of every file, counts
//! the bytes that reach the control plane, and searches the target and agent B for the canary.
//!
//! The test skips with a message when no restic runs on the machine, unless
//! `RESTICHUB_REQUIRE_RESTIC` asks it to fail.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;
mod restore_support;

use std::collections::BTreeMap;
use std::path::Path;

use axum::http::StatusCode;
use serde_json::json;

use queue_support::{Env, test_queue};
use restore_support::{
    RealAgent, audit_rows, finished_job, initialized_repository, real_restic_or_skip, restore_body,
    restore_uri, sha256_hex, snapshots_uri, work_dir,
};

const PASSWORD: &str = "CANARY-chain-restore-password";

fn restic(program: &Path, repository: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new(program)
        .args(args)
        .env("RESTIC_REPOSITORY", repository)
        .env("RESTIC_PASSWORD", PASSWORD)
        .env("RESTIC_CACHE_DIR", repository.with_extension("cache"))
        .output()
        .expect("restic must run");
    assert!(
        output.status.success(),
        "restic {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Every file below `root`, by its path below `root`, with its SHA-256.
fn hashes(root: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("inside the root")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, sha256_hex(&path));
            }
        }
    }
    files
}

fn contains_canary(root: &Path) -> bool {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).expect("a directory") {
                pending.push(entry.expect("an entry").path());
            }
        } else if std::fs::read(&path).is_ok_and(|bytes| {
            bytes
                .windows(PASSWORD.len())
                .any(|w| w == PASSWORD.as_bytes())
        }) {
            return true;
        }
    }
    false
}

/// AC-RS-03: the files on agent B agree with the source, the control plane relays under 1 % of the
/// data, agent B keeps no canary, and the audit log names the restore and each delivery.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rs_03_a_restore_through_the_control_plane_gives_equal_hashes_and_no_relay() {
    let Some(program) = real_restic_or_skip() else {
        return;
    };
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent_b = env.connect_agent("rs03-target").await;

    // Agent A: the source host writes the backup (the command line stands in for its agent).
    let work = work_dir("chain");
    let repository_path = work.join("repo");
    let data = work.join("source");
    std::fs::create_dir_all(data.join("deep/er")).expect("cannot create the tree");
    let mut random: u64 = 0x2545_f491_4f6c_dd1d;
    let mut payload = Vec::with_capacity(4 * 1024 * 1024);
    while payload.len() < 4 * 1024 * 1024 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        payload.extend_from_slice(&random.to_le_bytes());
    }
    std::fs::write(data.join("big.bin"), &payload).expect("cannot write the file");
    std::fs::write(data.join("deep/er/note.txt"), "a note").expect("cannot write the file");
    std::fs::write(data.join("zero.txt"), "").expect("cannot write the file");
    restic(&program, &repository_path, &["init", "--quiet"]);
    restic(
        &program,
        &repository_path,
        &["backup", "--quiet", data.to_str().expect("a path")],
    );
    let before = hashes(&data);

    // The repository is registered with agent B as its maintenance agent, so agent B may read it
    // (a repository with a local path accepts that agent only).
    let repository = initialized_repository(&env, agent_b.id, PASSWORD, &repository_path).await;
    let mut real_b = RealAgent::new("rs03-agent-b", &program);

    // The snapshot list shows the snapshot through the control plane.
    let (status, started) = env
        .call(
            &env.operator,
            "POST",
            &snapshots_uri(repository),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    env.pass().await;
    real_b.run_next(&mut agent_b).await;
    let uri = format!(
        "{}?job={}",
        snapshots_uri(repository),
        started["job_id"].as_str().expect("a job id")
    );
    queue_support::eventually(|| async {
        env.call(&env.operator, "GET", &uri, None).await.1["state"] == "done"
    })
    .await;
    let (_, list) = env.call(&env.operator, "GET", &uri, None).await;
    let snapshot = list["snapshots"][0]["id"]
        .as_str()
        .expect("a snapshot")
        .to_owned();
    assert_eq!(snapshot.len(), 64);

    // Agent B restores the snapshot from the repository.
    let target = work.join("restored");
    let (status, restore) = env
        .call(
            &env.operator,
            "POST",
            &restore_uri(repository),
            Some(restore_body(&snapshot, &target, json!({"verify": true}))),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{restore}");
    assert_eq!(restore["agent_id"], agent_b.id.to_string());
    env.pass().await;
    let sent_before = real_b.sent_bytes;
    let job = real_b.run_next(&mut agent_b).await;
    let ended = finished_job(&env, &job).await;
    assert_eq!(ended["status"], "succeeded", "{ended}");
    assert_eq!(ended["summary"]["kind"], "restore", "{ended}");
    assert!(
        ended["summary"]["files_restored"]
            .as_i64()
            .expect("a count")
            >= 3
    );
    assert_eq!(ended["summary"]["dry_run"], false);

    // The files on agent B have the SHA-256 values of the source.
    let restored_root = {
        let mut found = None;
        let mut pending = vec![target.clone()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).expect("a directory") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    if path.file_name() == data.file_name() {
                        found = Some(path.clone());
                    }
                    pending.push(path);
                }
            }
        }
        found.expect("the restored tree")
    };
    assert_eq!(
        hashes(&restored_root),
        before,
        "the SHA-256 of every file agrees"
    );

    // Under 1 % of the data volume reaches the control plane.
    let relayed = real_b.sent_bytes - sent_before;
    assert!(
        relayed * 100 < payload.len(),
        "{relayed} bytes reached the control plane for {} bytes of data",
        payload.len()
    );
    // The audit log names the restore, and the job delivered its own secrets only.
    let started = audit_rows(&env, "restore_start").await;
    assert_eq!(started.len(), 1, "{started:?}");
    assert!(
        started[0].contains(&agent_b.id.to_string()),
        "{}",
        started[0]
    );
    assert!(started[0].contains("restored"), "{}", started[0]);
    assert!(started[0].contains(&job), "{}", started[0]);
    let delivered = audit_rows(&env, "secret_delivered").await;
    assert!(
        delivered.iter().any(|row| row.contains(&job)),
        "{delivered:?}"
    );
    for row in started.iter().chain(&delivered) {
        assert!(!row.contains(PASSWORD), "no secret in the audit log: {row}");
    }

    // No canary reaches the target or agent B.
    assert!(!contains_canary(&target), "no canary in the restored files");
    assert!(
        !contains_canary(&real_b.dir),
        "no canary on agent B after the job"
    );
    let (_, log) = env
        .call(&env.admin, "GET", &format!("/api/v1/jobs/{job}/logs"), None)
        .await;
    assert!(
        !log.to_string().contains(PASSWORD),
        "no canary in the job log"
    );
    env.db.cleanup().await;
}
