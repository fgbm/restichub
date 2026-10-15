//! Certificate rotation over a real mTLS stream (FR-ID-03, AC-ID-03, ADR-0026).
//!
//! The control plane issues certificates with a lifetime of a few seconds, so the tests wait for
//! the half of the lifetime in real time. The rotation itself uses the checks of the real agent.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;
mod rotation_support;

use std::time::Duration;

use restichub_agent::rotation as agent_rotation;
use restichub_emulator::enroll::EnrolledIdentity;
use restichub_proto::agent::v1::{
    AgentMessage, CertificateInstalled, CertificateSigningRequest, ControlMessage, agent_message,
    control_message,
};

use queue_support::{Env, FakeAgent, eventually, next_message, test_queue};
use rotation_support::{
    audit_count, hex, rotated_identity, row, stored_identity, superseded_used_at,
};

/// How long a test certificate lives. The control plane accepts a request at half of it.
const LIFETIME: Duration = Duration::from_secs(6);

async fn start() -> Option<Env> {
    let db = common::test_db().await?;
    Some(Env::start_with_lifetime(db, test_queue(), LIFETIME).await)
}

/// Sends the request of a new key pair and returns the answer of the control plane.
async fn ask(agent: &mut FakeAgent) -> (agent_rotation::PendingRotation, ControlMessage) {
    let (pending, csr_der) = agent_rotation::begin().expect("a key pair");
    agent
        .tx
        .send(AgentMessage {
            payload: Some(agent_message::Payload::CertificateSigningRequest(
                CertificateSigningRequest { csr_der },
            )),
        })
        .await
        .expect("cannot send the request");
    loop {
        let message = next_message(&mut agent.inbound).await;
        if matches!(
            message.payload,
            Some(
                control_message::Payload::CertificateIssued(_)
                    | control_message::Payload::CertificateRejected(_)
            )
        ) {
            return (pending, message);
        }
    }
}

/// Waits in real time for half of the lifetime and moves the clock of the control plane to the
/// wall clock, as in production.
async fn wait_for_half_of_the_lifetime(env: &Env) {
    tokio::time::sleep(LIFETIME / 2 + Duration::from_millis(300)).await;
    env.clock.set(time::OffsetDateTime::now_utc());
}

async fn issued(enrolled: &EnrolledIdentity, agent: &mut FakeAgent) -> (EnrolledIdentity, Vec<u8>) {
    let (pending, message) = ask(agent).await;
    let Some(control_message::Payload::CertificateIssued(issued)) = message.payload else {
        panic!("the control plane must issue a certificate: {message:?}");
    };
    let now = time::OffsetDateTime::now_utc();
    let rotated = agent_rotation::accept(&stored_identity(enrolled), &pending, &issued, now)
        .expect("the agent must accept the certificate");
    let serial = agent_rotation::serial_of(&rotated.certificate_der).expect("a serial");
    (rotated_identity(enrolled, &rotated), serial)
}

/// AC-ID-03: the control plane signs a rotation request only after half of the lifetime, and the
/// old certificate stays the enrolled one until the agent confirms the new one.
#[tokio::test]
async fn ac_id_03_the_old_certificate_works_until_the_confirmation() {
    let Some(env) = start().await else {
        return;
    };
    let mut agent = env.connect_agent("stream-rotation-host").await;
    let enrolled = agent.identity.clone();
    let id = agent.id;
    let old_serial = hex(&agent_rotation::serial_of(&enrolled.certificate_der).expect("a serial"));

    let (_, early) = ask(&mut agent).await;
    assert!(
        matches!(
            early.payload,
            Some(control_message::Payload::CertificateRejected(_))
        ),
        "half of the lifetime has not passed: {early:?}"
    );

    wait_for_half_of_the_lifetime(&env).await;
    let (_, serial) = issued(&enrolled, &mut agent).await;
    let pending = row(&env, id).await;
    assert_eq!(pending.pending_serial, Some(hex(&serial)));
    assert_eq!(pending.serial, old_serial);
    assert_eq!(rotation_support::rotation_audit_count(&env).await, 1);

    // The old certificate still opens a stream, because the agent did not confirm.
    let (_tx, mut inbound) = env
        .open_raw(&enrolled)
        .await
        .expect("the old certificate works");
    assert!(inbound.message().await.is_ok());
    assert_eq!(row(&env, id).await.serial, old_serial);
    env.db.cleanup().await;
}

/// AC-ID-03: the confirmation makes the new certificate the enrolled one. The old serial is
/// refused from then on, the new certificate opens a stream, and a job that runs on the stream
/// during the rotation goes on.
#[tokio::test]
async fn ac_id_03_the_confirmation_refuses_the_old_certificate_and_a_job_continues() {
    let Some(env) = start().await else {
        return;
    };
    let mut agent = env.connect_agent("stream-confirm-host").await;
    let enrolled = agent.identity.clone();
    let id = agent.id;

    let repository = env.create_repository(id, "pw").await;
    let profile = env.create_profile(repository, serde_json::json!({})).await;
    let plan = env.create_plan(profile, id).await;
    let backup = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let job_id = queue_support::decode_job_id(&signed);
    assert_eq!(job_id, backup.to_string());
    agent.start(&env, &job_id).await;

    wait_for_half_of_the_lifetime(&env).await;
    let (renewed, serial) = issued(&enrolled, &mut agent).await;
    agent
        .tx
        .send(AgentMessage {
            payload: Some(agent_message::Payload::CertificateInstalled(
                CertificateInstalled {
                    certificate_serial: serial.clone(),
                },
            )),
        })
        .await
        .expect("cannot send the confirmation");
    eventually(|| async { row(&env, id).await.serial == hex(&serial) }).await;
    let promoted = row(&env, id).await;
    assert_eq!(promoted.pending_serial, None);
    assert!(promoted.last_rotation.is_some());

    // The job went on during the rotation, on the same stream.
    agent.finish(&env, &job_id).await;

    // The old certificate is refused for the next connection; the new one connects.
    let refused = env
        .open_raw(&enrolled)
        .await
        .expect_err("the old serial is refused");
    assert_eq!(refused.code(), tonic::Code::PermissionDenied, "{refused}");
    let connected = env.connect_enrolled(renewed).await;
    assert_eq!(connected.id, id);
    env.db.cleanup().await;
}

/// AC-ID-03: an agent that connects with the new certificate before it sent the confirmation
/// proves that it holds the new key; the control plane makes the certificate the enrolled one.
#[tokio::test]
async fn ac_id_03_a_connection_with_the_new_certificate_confirms_it() {
    let Some(env) = start().await else {
        return;
    };
    let mut agent = env.connect_agent("stream-implicit-host").await;
    let enrolled = agent.identity.clone();
    let id = agent.id;

    wait_for_half_of_the_lifetime(&env).await;
    let (renewed, serial) = issued(&enrolled, &mut agent).await;
    let (_tx, mut inbound) = env
        .open_raw(&renewed)
        .await
        .expect("the new certificate connects");
    assert!(inbound.message().await.is_ok());
    assert_eq!(row(&env, id).await.serial, hex(&serial));
    let refused = env
        .open_raw(&enrolled)
        .await
        .expect_err("the old serial is refused");
    assert_eq!(refused.code(), tonic::Code::PermissionDenied, "{refused}");
    env.db.cleanup().await;
}

/// FR-ID-03: the rate limit survives a reconnect. Ten connections that each send a request give
/// one issuance and one audit record; the others get `TooEarly`.
#[tokio::test]
async fn fr_id_03_ten_reconnects_with_a_request_give_one_issuance() {
    let Some(env) = start().await else {
        return;
    };
    let first = env.connect_agent("stream-reconnect-host").await;
    let enrolled = first.identity.clone();
    let id = first.id;
    wait_for_half_of_the_lifetime(&env).await;

    let mut issued_count = 0;
    let mut agent = first;
    for round in 0..10 {
        if round > 0 {
            agent = env.connect_enrolled(enrolled.clone()).await;
        }
        let (_, message) = ask(&mut agent).await;
        match message.payload {
            Some(control_message::Payload::CertificateIssued(_)) => issued_count += 1,
            Some(control_message::Payload::CertificateRejected(rejected)) => assert_eq!(
                rejected.reason,
                restichub_proto::agent::v1::CertificateRejectReason::TooEarly as i32,
                "round {round}"
            ),
            other => panic!("round {round}: {other:?}"),
        }
    }
    assert_eq!(issued_count, 1);
    assert_eq!(rotation_support::rotation_audit_count(&env).await, 1);
    assert!(row(&env, id).await.pending_serial.is_some());
    env.db.cleanup().await;
}

/// T17: a connection with the serial that a rotation replaced is refused, writes one audit event
/// for the quiet period and marks the host.
#[tokio::test]
async fn t17_a_superseded_certificate_writes_an_audit_event_and_marks_the_host() {
    let Some(env) = start().await else {
        return;
    };
    let mut agent = env.connect_agent("stream-clone-host").await;
    let enrolled = agent.identity.clone();
    let id = agent.id;
    wait_for_half_of_the_lifetime(&env).await;
    let (renewed, serial) = issued(&enrolled, &mut agent).await;
    let (_tx, mut inbound) = env.open_raw(&renewed).await.expect("the new certificate");
    assert!(inbound.message().await.is_ok());
    assert_eq!(row(&env, id).await.serial, hex(&serial));
    assert!(superseded_used_at(&env, id).await.is_none());

    for _ in 0..3 {
        let refused = env
            .open_raw(&enrolled)
            .await
            .expect_err("the old serial is refused");
        assert_eq!(refused.code(), tonic::Code::PermissionDenied, "{refused}");
    }
    assert_eq!(
        audit_count(&env, "agent_superseded_certificate_used").await,
        1
    );
    assert!(superseded_used_at(&env, id).await.is_some());
    env.db.cleanup().await;
}
