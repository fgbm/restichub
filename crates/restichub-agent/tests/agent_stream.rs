//! The agent connect loop against the real control-plane listener (FR-ID-04, FR-ID-07, FR-PR-11).
//!
//! The tests start the real agent listener with the installation CA, enroll the real agent over
//! HTTPS, open the stream and drive the protocol. They cover the heartbeat, the `last_seen_at`
//! write, the key rotation and its acknowledgement, and the revoke of the identity.

#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use restichub_agent::cli::RegisterOptions;
use restichub_agent::commands;
use restichub_agent::identity::StoredIdentity;
use restichub_agent::restic::Restic;
use restichub_agent::stream::{self, Backoff, Disconnect};
use restichub_db::{Database, TenantId};
use tokio::sync::watch;
use uuid::Uuid;

/// A restic double. The hash check has its own unit tests (AC-AG-11).
struct FakeRestic;

impl Restic for FakeRestic {
    fn verify(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn version(&self) -> anyhow::Result<String> {
        Ok("0.19.1".to_owned())
    }
}

/// Enrolls the real agent against the listener and returns the configuration directory.
async fn register(fixture: &common::StreamFixture, dir: &std::path::Path) -> StoredIdentity {
    let ca_file = common::write_temp_file("stream-ca.pem", fixture.server.root_pem().as_bytes());
    let options = RegisterOptions {
        url: Some(fixture.server.endpoint.clone()),
        token: Some(fixture.token.clone()),
        ca_file: Some(ca_file),
        ..RegisterOptions::default()
    };
    commands::register_async(&options, dir, &FakeRestic)
        .await
        .expect("the agent must register");
    StoredIdentity::load(dir).expect("the identity must load")
}

/// Waits until `last_seen_at` moves past `after`.
async fn wait_for_last_seen(
    db: &Database,
    tenant: TenantId,
    agent: Uuid,
    after: time::OffsetDateTime,
) -> bool {
    for _ in 0..200 {
        if let Some(seen) = common::last_seen(db, tenant, agent).await
            && seen > after
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Waits until the control plane retires an old signing key.
async fn wait_for_retire(db: &Database) -> bool {
    for _ in 0..200 {
        if common::retired_keys(db).await > 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn fast_backoff() -> Backoff {
    Backoff {
        base: Duration::from_millis(20),
        max: Duration::from_millis(100),
    }
}

/// AC-ID-07 / FR-ID-07 / FR-PR-11: the agent enrolls, connects, writes `last_seen_at` and
/// acknowledges a key rotation.
#[tokio::test(flavor = "multi_thread")]
async fn ac_id_07_the_agent_connects_and_acknowledges_a_key_rotation() {
    let Some(fixture) = common::stream_fixture("agent-stream").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-stream");
    let identity = register(&fixture, &dir).await;
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");

    // Move `last_seen_at` into the past, so only the stream can move it forward.
    let old = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    common::set_last_seen(&fixture.db, fixture.tenant, agent, old).await;

    let mut run_identity = StoredIdentity::load(&dir).expect("the identity must load");
    let mut ring = stream::key_ring(&run_identity).expect("the key ring must build");
    let dir_task = dir.clone();
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        stream::run(
            &mut run_identity,
            &dir_task,
            &mut ring,
            "0.19.1",
            fast_backoff(),
            &mut stop_rx,
        )
        .await
    });

    assert!(
        wait_for_last_seen(fixture.db.database(), fixture.tenant, agent, old).await,
        "the stream must write last_seen_at"
    );

    // Rotate the key set. The stream delivers the new set and the agent acknowledges it.
    let signing = fixture
        .server
        .state
        .job_signing()
        .expect("the job signing service is loaded")
        .clone();
    signing
        .rotate(fixture.db.database(), time::OffsetDateTime::now_utc())
        .await
        .expect("the rotation must succeed");
    assert!(
        wait_for_retire(fixture.db.database()).await,
        "the agent must acknowledge the rotation, so the old key retires"
    );

    // Stop the agent as the service does.
    stop_tx.send(true).expect("cannot stop the agent");
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the agent must stop")
        .expect("the agent must not panic");
    assert!(result.is_ok(), "the loop must stop without an error");

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

/// AC-ID-04 / FR-ID-04: a revoke closes the open stream and the agent is refused at the next
/// attempt, so it backs off at the maximum.
#[tokio::test(flavor = "multi_thread")]
async fn ac_id_04_a_revoke_closes_the_stream_and_the_agent_is_refused() {
    let Some(fixture) = common::stream_fixture("agent-revoke").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-revoke");
    let identity = register(&fixture, &dir).await;
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");

    let old = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    common::set_last_seen(&fixture.db, fixture.tenant, agent, old).await;

    // Open one stream in the background.
    let mut run_identity = StoredIdentity::load(&dir).expect("the identity must load");
    let mut ring = stream::key_ring(&run_identity).expect("the key ring must build");
    let dir_task = dir.clone();
    let (_stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        stream::connect_once(
            &mut run_identity,
            &dir_task,
            &mut ring,
            "0.19.1",
            &mut stop_rx,
        )
        .await
    });
    assert!(
        wait_for_last_seen(fixture.db.database(), fixture.tenant, agent, old).await,
        "the stream must open and write last_seen_at"
    );

    // Revoke the agent; the open stream closes at once (FR-ID-04).
    let actor = common::create_user(
        fixture.db.database(),
        fixture.tenant,
        "revoker",
        "phc",
        "admin",
    )
    .await;
    fixture
        .server
        .revoke(fixture.db.database(), fixture.tenant, agent, actor)
        .await;
    let outcome = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the revoke must close the stream")
        .expect("the stream must not panic")
        .expect("the stream must not fail");
    assert_eq!(outcome, Disconnect::Closed);

    // Open a fresh attempt; the registry check refuses it.
    let mut identity = StoredIdentity::load(&dir).expect("the identity must load");
    let mut ring = stream::key_ring(&identity).expect("the key ring must build");
    let (_keep, mut stop_rx) = watch::channel(false);
    let outcome = stream::connect_once(&mut identity, &dir, &mut ring, "0.19.1", &mut stop_rx)
        .await
        .expect("the attempt must not fail at the transport");
    assert_eq!(outcome, Disconnect::Refused);

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

/// FR-ID-08: the backoff keeps the agent in the loop while the identity is refused.
#[tokio::test(flavor = "multi_thread")]
async fn fr_id_08_a_refused_identity_keeps_the_agent_in_the_loop() {
    let Some(fixture) = common::stream_fixture("agent-refused").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-refused");
    let identity = register(&fixture, &dir).await;
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");

    // Revoke before the first connect, so every attempt is refused.
    let actor = common::create_user(
        fixture.db.database(),
        fixture.tenant,
        "revoker",
        "phc",
        "admin",
    )
    .await;
    fixture
        .server
        .revoke(fixture.db.database(), fixture.tenant, agent, actor)
        .await;

    let mut run_identity = StoredIdentity::load(&dir).expect("the identity must load");
    let mut ring = stream::key_ring(&run_identity).expect("the key ring must build");
    let dir_task = dir.clone();
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        stream::run(
            &mut run_identity,
            &dir_task,
            &mut ring,
            "0.19.1",
            fast_backoff(),
            &mut stop_rx,
        )
        .await
    });

    // Keep the loop alive; it retries instead of exiting.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!task.is_finished(), "the loop must keep retrying");
    stop_tx.send(true).expect("cannot stop the agent");
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the agent must stop")
        .expect("the agent must not panic");
    assert!(result.is_ok());

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}
