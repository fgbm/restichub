//! Protocol version negotiation at the start of a stream (FR-PR-09, AC-PR-06).
//!
//! The control plane accepts the version N and the previous minor version N-1, and it refuses the
//! next major version with a typed reason. The registry keeps the version of the last hello, and
//! the host list marks an agent that the control plane refuses.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use queue_support::{Env, eventually, next_message, test_queue};
use restichub_emulator::enroll::EnrolledIdentity;
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, ControlMessage, VersionRejectReason, agent_message, control_message,
};
use restichub_proto::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};
use serde_json::Value;
use tonic::Streaming;

fn hello(major: u32, minor: u32, agent_version: &str) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::AgentHello(AgentHello {
            protocol_version_major: major,
            protocol_version_minor: minor,
            agent_version: agent_version.to_owned(),
            restic_version: "0.19.1".to_owned(),
            ..AgentHello::default()
        })),
    }
}

/// Opens a stream with the given version and reads the answer of the control plane: the version
/// refusal, or `None` when the greeting arrived without one.
async fn negotiate(
    env: &Env,
    identity: &EnrolledIdentity,
    major: u32,
    minor: u32,
) -> (
    Option<restichub_proto::agent::v1::VersionRejected>,
    Streaming<ControlMessage>,
) {
    let (tx, mut inbound) = env.open_raw(identity).await.expect("the stream must open");
    tx.send(hello(major, minor, "7.1.0"))
        .await
        .expect("cannot send the hello");
    // Keep the sender alive for the test, so the stream does not end by itself.
    std::mem::forget(tx);
    let mut rejected = None;
    // The refusal follows the greeting and the key set. A compatible stream stays silent after
    // them, so a short wait without a message ends the loop.
    while let Ok(Ok(Some(message))) =
        tokio::time::timeout(Duration::from_millis(1500), inbound.message()).await
    {
        match message.payload {
            Some(control_message::Payload::VersionRejected(refusal)) => {
                rejected = Some(refusal);
                break;
            }
            Some(control_message::Payload::ServerHello(greeting)) => {
                assert_eq!(greeting.protocol_version_major, PROTOCOL_VERSION_MAJOR);
                assert_eq!(greeting.protocol_version_minor, PROTOCOL_VERSION_MINOR);
                assert!(!greeting.control_plane_version.is_empty());
            }
            _ => {}
        }
    }
    (rejected, inbound)
}

async fn host(env: &Env, id: &str) -> Value {
    let (status, _, body) = common::send(
        &env.app,
        common::bearer_request("GET", &format!("/api/v1/agents/{id}"), &env.admin, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// AC-PR-06: version N and version N-1 work, a newer minor version works too, and the next major
/// version is refused with a typed reason. The refused agent never counts as online.
#[tokio::test]
async fn ac_pr_06_the_current_and_the_previous_minor_work_and_the_next_major_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;

    for (name, major, minor) in [
        ("current", PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR),
        (
            "previous-minor",
            PROTOCOL_VERSION_MAJOR,
            PROTOCOL_VERSION_MINOR.saturating_sub(1),
        ),
        (
            "newer-minor",
            PROTOCOL_VERSION_MAJOR,
            PROTOCOL_VERSION_MINOR + 1,
        ),
    ] {
        let identity = env.enroll_agent(name).await;
        let (rejected, _inbound) = negotiate(&env, &identity, major, minor).await;
        assert!(rejected.is_none(), "{name} must work");
        let id = identity.agent_id.clone();
        let agent = restichub_db::AgentId::from_uuid(id.parse().expect("agent id"));
        eventually(|| async { env.state.agent_registry().is_online(agent) }).await;
        let facts = host(&env, &id).await;
        assert_eq!(
            facts["protocol_version"],
            format!("{major}.{minor}"),
            "{name}"
        );
        assert_eq!(
            facts["agent_version"], "7.1.0",
            "the hello updates the version"
        );
        assert_eq!(facts["version_status"], "current", "{name}");
    }

    let identity = env.enroll_agent("next-major").await;
    let agent = restichub_db::AgentId::from_uuid(identity.agent_id.parse().expect("agent id"));
    let (rejected, mut inbound) = negotiate(&env, &identity, PROTOCOL_VERSION_MAJOR + 1, 0).await;
    let rejected = rejected.expect("the next major version is refused");
    assert_eq!(
        VersionRejectReason::try_from(rejected.reason).expect("a reason"),
        VersionRejectReason::MajorMismatch
    );
    assert_eq!(
        rejected.server_protocol_version_major,
        PROTOCOL_VERSION_MAJOR
    );
    assert_eq!(
        rejected.server_protocol_version_minor,
        PROTOCOL_VERSION_MINOR
    );
    // The control plane ends the stream after the refusal.
    let ended = tokio::time::timeout(Duration::from_secs(5), inbound.message())
        .await
        .expect("the stream ends");
    assert!(matches!(ended, Ok(None) | Err(_)), "{ended:?}");
    assert!(!env.state.agent_registry().is_online(agent));

    let facts = host(&env, &identity.agent_id).await;
    assert_eq!(
        facts["protocol_version"],
        format!("{}.0", PROTOCOL_VERSION_MAJOR + 1)
    );
    assert_eq!(facts["version_status"], "unsupported");
    env.db.cleanup().await;
}

/// FR-PR-09: a hello that names no version text keeps the version of the enrollment, and a hello
/// with a control character in the version drops it.
#[tokio::test]
async fn fr_pr_09_the_registry_keeps_a_clean_version_text() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let identity = env.enroll_agent("clean-version").await;
    let (tx, mut inbound) = env.open_raw(&identity).await.expect("the stream must open");
    tx.send(hello(
        PROTOCOL_VERSION_MAJOR,
        PROTOCOL_VERSION_MINOR,
        "7.1.0\u{1b}[31m",
    ))
    .await
    .expect("cannot send the hello");
    let agent = restichub_db::AgentId::from_uuid(identity.agent_id.parse().expect("agent id"));
    loop {
        let message: ControlMessage = next_message(&mut inbound).await;
        if matches!(
            message.payload,
            Some(control_message::Payload::SigningKeyUpdate(_))
        ) {
            break;
        }
    }
    eventually(|| async { env.state.agent_registry().is_online(agent) }).await;
    let facts = host(&env, &identity.agent_id).await;
    assert_eq!(
        facts["agent_version"], "0.1.0",
        "the enrollment version stays"
    );
    drop(tx);
    env.db.cleanup().await;
}
