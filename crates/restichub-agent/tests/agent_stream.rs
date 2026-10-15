//! The agent connect loop against the real control-plane listener (FR-ID-04, FR-ID-07, FR-PR-11).
//!
//! The tests start the real agent listener with the installation CA, enroll the real agent over
//! HTTPS, open the stream and drive the protocol. They cover the heartbeat, the `last_seen_at`
//! write, the key rotation and its acknowledgement, the revoke of the identity, the reconnect
//! backoff, the inbound idle timeout and the key-update chain of a lagging agent (ADR-0030).

#![allow(clippy::expect_used)]

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use restichub_agent::cli::RegisterOptions;
use restichub_agent::commands;
use restichub_agent::identity::StoredIdentity;
use restichub_agent::keys::KeyState;
use restichub_agent::restic::Restic;
use restichub_agent::stream::{self, Backoff, Disconnect, StreamOptions, Timeouts};
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

/// Enrolls the real agent against the listener and returns the stored identity.
async fn register(fixture: &common::StreamFixture, dir: &Path) -> StoredIdentity {
    let ca_file = common::write_temp_file("stream-ca.pem", fixture.server.root_pem().as_bytes());
    let options = RegisterOptions {
        url: Some(fixture.server.endpoint.clone()),
        token: Some(fixture.token.clone()),
        ca_file: Some(ca_file),
        ..RegisterOptions::default()
    };
    commands::register_async(&options, dir, &dir.with_extension("cache"), &FakeRestic)
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

/// Waits until the persisted chain brings the ring to `key_id` (ADR-0030).
async fn wait_for_key(identity: &StoredIdentity, dir: &Path, key_id: &str) -> bool {
    for _ in 0..200 {
        if let Ok(keys) = KeyState::load(identity, dir)
            && keys.ring().get(key_id).is_some()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// A fast backoff with a fixed jitter, so a test is deterministic (FR-ID-08).
fn fast_backoff() -> Backoff {
    Backoff {
        base: Duration::from_millis(50),
        max: Duration::from_millis(400),
        fraction: Some(0.5),
    }
}

/// The reconnect parameters for the normal tests.
fn options() -> StreamOptions {
    StreamOptions {
        backoff: fast_backoff(),
        timeouts: Timeouts::default(),
    }
}

/// Timeouts that make the heartbeat and the idle close quick, for the silent-server test (FR-ID-07).
fn short_timeouts() -> Timeouts {
    Timeouts {
        ping: Duration::from_millis(100),
        connect: Duration::from_secs(5),
        keepalive_interval: Duration::from_secs(5),
        keepalive_timeout: Duration::from_secs(5),
        tcp_keepalive: Duration::from_secs(5),
        ..Timeouts::default()
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
    let identity = Arc::new(register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");

    // Move `last_seen_at` into the past, so only the stream can move it forward.
    let old = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    common::set_last_seen(&fixture.db, fixture.tenant, agent, old).await;

    let dir_task = dir.clone();
    let identity_task = Arc::clone(&identity);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity_task, &dir_task).expect("the key state must load");
        let jobs = common::unused_jobs(&dir_task);
        stream::run(
            &identity_task,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            options(),
            &mut stop_rx,
        )
        .await
    });

    assert!(
        wait_for_last_seen(fixture.db.database(), fixture.tenant, agent, old).await,
        "the stream must write last_seen_at"
    );
    let mut connected = false;
    for _ in 0..200 {
        connected = restichub_agent::connection::ConnectionStatus::read(&dir)
            .ok()
            .flatten()
            .is_some_and(|status| status.connected);
        if connected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        connected,
        "the status must show the connection after the greeting (FR-IN-07)"
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
    let mut keys = KeyState::load(&identity, &dir).expect("the key state must load");
    let dir_task = dir.clone();
    let identity_task = identity;
    let (_stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let jobs = common::unused_jobs(&dir_task);
        stream::connect_once(
            &identity_task,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            Timeouts::default(),
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
    assert_eq!(outcome, Disconnect::Closed { healthy: false });

    // Open a fresh attempt; the registry check refuses it.
    let identity = StoredIdentity::load(&dir).expect("the identity must load");
    let mut keys = KeyState::load(&identity, &dir).expect("the key state must load");
    let jobs = common::unused_jobs(&dir);
    let (_keep, mut stop_rx) = watch::channel(false);
    let outcome = stream::connect_once(
        &identity,
        &dir,
        &mut keys,
        "0.19.1",
        &jobs,
        Timeouts::default(),
        &mut stop_rx,
    )
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
    let identity = Arc::new(register(&fixture, &dir).await);
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

    let dir_task = dir.clone();
    let identity_task = Arc::clone(&identity);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity_task, &dir_task).expect("the key state must load");
        let jobs = common::unused_jobs(&dir_task);
        stream::run(
            &identity_task,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            options(),
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

/// a server that closes right after the open gives growing delays, so the agent does not hammer
/// it. The fixed jitter makes the count deterministic.
#[tokio::test(flavor = "multi_thread")]
async fn m2_a_server_that_closes_at_once_grows_the_delay() {
    let Some(fixture) = common::stream_fixture("agent-closing").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-closing");
    let mut identity = register(&fixture, &dir).await;
    let server =
        common::SilentServer::start_closing(fixture.server.ca(), fixture.server.root_der()).await;
    identity.grpc_endpoint = Some(server.endpoint());

    let identity = Arc::new(identity);
    let dir_task = dir.clone();
    let identity_task = Arc::clone(&identity);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity_task, &dir_task).expect("the key state must load");
        let jobs = common::unused_jobs(&dir_task);
        stream::run(
            &identity_task,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            options(),
            &mut stop_rx,
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(700)).await;
    let attempts = server.connections();
    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(attempts >= 2, "the agent must retry: {attempts}");
    assert!(
        attempts <= 10,
        "the delay must grow, not reset, so the attempts stay low: {attempts}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

/// FR-ID-08: a stream that answered a ping and then failed (a control plane restart) resets the
/// reconnect attempt, unlike a stream that closed at once.
#[tokio::test(flavor = "multi_thread")]
async fn fr_id_08_a_healthy_stream_that_fails_resets_the_attempt() {
    let Some(fixture) = common::stream_fixture("agent-healthy-fail").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-healthy-fail");
    let mut identity = register(&fixture, &dir).await;
    let server =
        common::SilentServer::start_ping_then_error(fixture.server.ca(), fixture.server.root_der())
            .await;
    identity.grpc_endpoint = Some(server.endpoint());

    let mut keys = KeyState::load(&identity, &dir).expect("the key state must load");
    let jobs = common::unused_jobs(&dir);
    let (_stop_tx, mut stop_rx) = watch::channel(false);
    let outcome = stream::connect_once(
        &identity,
        &dir,
        &mut keys,
        "0.19.1",
        &jobs,
        short_timeouts(),
        &mut stop_rx,
    )
    .await
    .expect("the transport must not fail");
    assert_eq!(
        outcome,
        Disconnect::Closed { healthy: true },
        "a stream that answered a ping is healthy, so the attempt resets"
    );

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

/// a server that stops answering makes the agent reconnect after three ping intervals.
#[tokio::test(flavor = "multi_thread")]
async fn m5_a_silent_server_makes_the_agent_reconnect() {
    let Some(fixture) = common::stream_fixture("agent-silent").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-silent");
    let mut identity = register(&fixture, &dir).await;
    let server = common::SilentServer::start(fixture.server.ca(), fixture.server.root_der()).await;
    identity.grpc_endpoint = Some(server.endpoint());

    let identity = Arc::new(identity);
    let dir_task = dir.clone();
    let identity_task = Arc::clone(&identity);
    let options = StreamOptions {
        backoff: fast_backoff(),
        timeouts: short_timeouts(),
    };
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity_task, &dir_task).expect("the key state must load");
        let jobs = common::unused_jobs(&dir_task);
        stream::run(
            &identity_task,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            options,
            &mut stop_rx,
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(1200)).await;
    let connections = server.connections();
    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(
        connections >= 2,
        "the agent must reconnect after the idle timeout: {connections}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

/// ADR-0030: an agent that missed two rotations reconnects, applies the chain and ends on the
/// current key.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0030_an_agent_that_missed_two_rotations_catches_up() {
    let Some(fixture) = common::stream_fixture("agent-catchup").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-catchup");
    let identity = Arc::new(register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let signing = fixture
        .server
        .state
        .job_signing()
        .expect("the job signing service is loaded")
        .clone();

    // The agent connects once, so it holds the enrollment set, then goes offline.
    let old = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    common::set_last_seen(&fixture.db, fixture.tenant, agent, old).await;
    let dir_task = dir.clone();
    let identity_task = Arc::clone(&identity);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let first = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity_task, &dir_task).expect("the key state must load");
        let jobs = common::unused_jobs(&dir_task);
        stream::run(
            &identity_task,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            options(),
            &mut stop_rx,
        )
        .await
    });
    assert!(
        wait_for_last_seen(fixture.db.database(), fixture.tenant, agent, old).await,
        "the first stream must open"
    );
    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), first).await;

    // Two rotations happen while the agent is offline. The overlap of each rotation is retired, so
    // the next rotation is allowed.
    let now = time::OffsetDateTime::now_utc();
    let db = fixture.db.database();
    signing.rotate(db, now).await.expect("the first rotation");
    signing
        .retire_due(db, now + time::Duration::days(8))
        .await
        .expect("the first retirement");
    signing
        .rotate(db, now + time::Duration::days(8))
        .await
        .expect("the second rotation");
    signing
        .retire_due(db, now + time::Duration::days(16))
        .await
        .expect("the second retirement");
    let current = common::current_key_id(db).await;

    // The agent reconnects and applies the chain. The control plane sends the whole chain on the
    // connect, so the agent reaches the current key.
    let dir_task = dir.clone();
    let identity_task = Arc::clone(&identity);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let second = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity_task, &dir_task).expect("the key state must load");
        let jobs = common::unused_jobs(&dir_task);
        stream::run(
            &identity_task,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            options(),
            &mut stop_rx,
        )
        .await
    });
    assert!(
        wait_for_key(&identity, &dir, &current).await,
        "the agent must reach the current key"
    );
    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), second).await;

    let keys = KeyState::load(&identity, &dir).expect("the key state must load");
    assert!(
        keys.ring().get(&current).is_some(),
        "the persisted chain must hold the current key"
    );

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}
