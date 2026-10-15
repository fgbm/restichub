//! The real agent rotates its certificate against the real control plane (FR-ID-03, AC-ID-03).
//!
//! The control plane issues certificates with a lifetime of a few seconds. The agent asks at two
//! thirds of the lifetime over its open stream, stores the answer next to the rest of its state and
//! asks again for the new certificate. The stream stays open through the rotations.

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
use restichub_agent::stream::{self, Backoff, StreamOptions, Timeouts};
use tokio::sync::watch;

/// The lifetime of a test certificate. The agent asks after six seconds; the control plane accepts
/// a request after three.
const LIFETIME: Duration = Duration::from_secs(9);

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
    let ca_file = common::write_temp_file("rotation-ca.pem", fixture.server.root_pem().as_bytes());
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

/// Waits until the stored certificate is not the given one.
async fn wait_for_new_certificate(dir: &Path, old: &[u8], wait: Duration) -> StoredIdentity {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if let Ok(identity) = StoredIdentity::load(dir)
            && identity.certificate_der != old
        {
            return identity;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the agent must rotate its certificate in time"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// AC-ID-03: the agent rotates at two thirds of the lifetime, stores the new key and certificate,
/// rotates again for the new certificate, and its stream stays open. After the rotations the
/// control plane accepts the new certificate and no longer the first one.
#[tokio::test(flavor = "multi_thread")]
async fn ac_id_03_the_agent_rotates_twice_over_its_open_stream() {
    let Some(fixture) =
        common::stream_fixture_with_lifetime("agent-rotation", Some(LIFETIME)).await
    else {
        return;
    };
    let dir = common::temp_config_dir("agent-rotation");
    let identity = Arc::new(register(&fixture, &dir).await);
    let first_expiry = identity.certificate_expiry().expect("an expiry");
    let (first_not_before, _) = identity.certificate_validity().expect("a validity");
    let first_key = identity.key_pem.clone();

    let dir_task = dir.clone();
    let identity_task = Arc::clone(&identity);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity_task, &dir_task).expect("the key state");
        let jobs = common::unused_jobs(&dir_task);
        let options = StreamOptions {
            backoff: Backoff {
                base: Duration::from_millis(50),
                max: Duration::from_millis(400),
                fraction: Some(0.5),
            },
            timeouts: Timeouts {
                rotation_retry: Duration::from_millis(500),
                ..Timeouts::default()
            },
        };
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

    let started = tokio::time::Instant::now();
    let second =
        wait_for_new_certificate(&dir, &identity.certificate_der, Duration::from_secs(15)).await;
    let first_rotation = started.elapsed();
    assert!(
        first_rotation >= Duration::from_secs(3),
        "the agent asked after half of the lifetime: {first_rotation:?}"
    );
    assert_ne!(second.key_pem, first_key, "a rotation renews the key pair");
    assert!(second.certificate_expiry().expect("an expiry") > first_expiry);
    let (second_not_before, _) = second.certificate_validity().expect("a validity");
    assert!(second_not_before > first_not_before);
    assert_eq!(second.agent_id, identity.agent_id);
    assert_eq!(second.ca_chain_der.last(), identity.ca_chain_der.last());

    // The new certificate rotates in its turn, on the same connection.
    let third =
        wait_for_new_certificate(&dir, &second.certificate_der, Duration::from_secs(15)).await;
    assert_ne!(third.key_pem, second.key_pem);
    assert!(
        third.certificate_expiry().expect("an expiry")
            > second.certificate_expiry().expect("an expiry")
    );

    let agent = uuid::Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let rotated_at = common::last_rotation(fixture.db.database(), fixture.tenant, agent).await;
    assert!(
        rotated_at.is_some(),
        "the control plane recorded the rotation"
    );

    // The files of one identity belong together: the stored key opens the stored certificate.
    let reloaded = StoredIdentity::load(&dir).expect("the identity reloads");
    assert_eq!(reloaded.certificate_der, third.certificate_der);
    assert_eq!(reloaded.key_pem, third.key_pem);

    stop_tx.send(true).expect("cannot stop the agent");
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the agent must stop")
        .expect("the agent must not panic");
    assert!(result.is_ok(), "the loop must stop without an error");
    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}
