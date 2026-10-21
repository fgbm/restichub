//! The agent connect attempt against a control plane that accepts TCP and never answers the TLS
//! handshake (FR-ID-08).

#![allow(clippy::expect_used)]

mod common;

use std::path::Path;
use std::time::Duration;

use restichub_agent::cli::RegisterOptions;
use restichub_agent::commands;
use restichub_agent::identity::StoredIdentity;
use restichub_agent::keys::KeyState;
use restichub_agent::restic::Restic;
use restichub_agent::stream::{self, Backoff, StreamOptions, Timeouts};
use tokio::sync::watch;

struct FakeRestic;

impl Restic for FakeRestic {
    fn verify(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn version(&self) -> anyhow::Result<String> {
        Ok("0.19.1".to_owned())
    }
}

async fn register(fixture: &common::StreamFixture, dir: &Path) -> StoredIdentity {
    let ca_file = common::write_temp_file("handshake-ca.pem", fixture.server.root_pem().as_bytes());
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

/// Accepts TCP connections and holds them without a byte, as a control plane whose connections
/// wait in the listen backlog.
async fn mute_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let held = tokio::spawn(async move {
        let mut sockets = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            sockets.push(socket);
        }
    });
    (format!("https://localhost:{}", address.port()), held)
}

#[tokio::test(flavor = "multi_thread")]
async fn fr_id_08_a_server_that_never_answers_ends_the_connect_attempt() {
    let Some(fixture) = common::stream_fixture("agent-mute-handshake").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-mute-handshake");
    let mut identity = register(&fixture, &dir).await;
    let (endpoint, held) = mute_server().await;
    identity.grpc_endpoint = Some(endpoint);
    let mut keys = KeyState::load(&identity, &dir).expect("the key state must load");
    let jobs = common::unused_jobs(&dir);
    let (_stop_tx, mut stop_rx) = watch::channel(false);
    let timeouts = Timeouts {
        connect: Duration::from_millis(300),
        ..Timeouts::default()
    };

    let attempt = tokio::time::timeout(
        Duration::from_secs(5),
        stream::connect_once(
            &identity,
            &dir,
            &mut keys,
            "0.19.1",
            &jobs,
            timeouts,
            &mut stop_rx,
        ),
    )
    .await
    .expect("the attempt must end on the connect timeout");
    assert!(attempt.is_err(), "a handshake that never ends is a failure");

    held.abort();
    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn fr_id_08_a_stop_ends_a_pending_handshake() {
    let Some(fixture) = common::stream_fixture("agent-stop-handshake").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-stop-handshake");
    let mut identity = register(&fixture, &dir).await;
    let (endpoint, held) = mute_server().await;
    identity.grpc_endpoint = Some(endpoint);
    let dir_task = dir.clone();
    let options = StreamOptions {
        backoff: Backoff::default(),
        timeouts: Timeouts {
            connect: Duration::from_secs(60),
            ..Timeouts::default()
        },
    };
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity, &dir_task).expect("the key state must load");
        let jobs = common::unused_jobs(&dir_task);
        stream::run(
            &identity,
            &dir_task,
            &mut keys,
            "0.19.1",
            &jobs,
            options,
            &mut stop_rx,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!task.is_finished(), "the handshake must still wait");

    stop_tx.send(true).expect("cannot stop the agent");
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("a stop must not wait for the handshake")
        .expect("the agent must not panic");
    assert!(result.is_ok(), "the loop must stop without an error");

    held.abort();
    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}
