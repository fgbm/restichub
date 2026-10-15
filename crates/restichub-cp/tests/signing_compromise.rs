//! The compromise of the job signing key (ADR-0030, FR-PR-11, AC-PR-09, T13).
//!
//! The action rotates the key set and retires the old current key with the reason `compromise`.
//! The signed update revokes the key on every agent that holds the next key, and the re-sync chain
//! never carries the compromised key.

#![allow(clippy::expect_used)]

mod common;
mod pki_support;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use ed25519_dalek::{SigningKey, VerifyingKey};
use prost_types::Timestamp;
use restichub_cp::agent_stream::REVOCATION_SWEEP_INTERVAL;
use restichub_cp::job_signing::JobSigningService;
use restichub_db::audit::{AuditAction, AuditEvent};
use restichub_db::signing;
use restichub_db::{PLATFORM_TENANT_ID, SigningKeyRole};
use restichub_proto::agent::v1::{Job, control_message};
use restichub_proto::signing::{Expected, KeyRing, verify_job};
use restichub_secrets::{WrappedSigningKey, unwrap_signing_key};
use time::OffsetDateTime;
use uuid::Uuid;

use pki_support::{provider, test_db};
use queue_support::{Env, next_message, test_queue};

fn job() -> Job {
    Job {
        job_id: "job-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        issued_at: Some(Timestamp {
            seconds: 900,
            nanos: 0,
        }),
        expires_at: Some(Timestamp {
            seconds: 2_000,
            nanos: 0,
        }),
        ..Job::default()
    }
}

fn expected() -> Expected {
    Expected {
        agent_id: "agent-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        now: Timestamp {
            seconds: 1_000,
            nanos: 0,
        },
    }
}

fn event() -> AuditEvent {
    AuditEvent::new(AuditAction::SigningKeyCompromise, "signing_key")
}

/// The public key of the active key in a role, read through the stored rows and the KEK.
async fn public_key(db: &restichub_db::Database, role: SigningKeyRole) -> (String, VerifyingKey) {
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin");
    let rows = signing::list(&mut tx).await.expect("cannot list the keys");
    let row = rows
        .into_iter()
        .find(|row| row.role == role && row.retired_at.is_none() && row.retire_after.is_none())
        .expect("the active key is present");
    let wrapped = WrappedSigningKey {
        kek_id: restichub_secrets::KekId::parse(&row.kek_id).expect("a KEK id"),
        nonce: row.nonce.as_slice().try_into().expect("a nonce"),
        ciphertext: row.wrapped_key.clone(),
    };
    let seed = unwrap_signing_key(
        provider().as_ref(),
        PLATFORM_TENANT_ID.as_uuid(),
        row.id,
        row.role.as_str(),
        &wrapped,
    )
    .expect("the KEK opens the key");
    let seed: [u8; 32] = seed.as_slice().try_into().expect("32 bytes");
    (
        row.id.to_string(),
        SigningKey::from_bytes(&seed).verifying_key(),
    )
}

async fn audit_count(env_db: &restichub_db::test_util::TestDatabase, action: &str) -> i64 {
    let mut tx = env_db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the scope");
    sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = $1")
        .bind(action)
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count")
}

/// ADR-0030: the compromise retires the old current key at once with the reason `compromise`, the
/// next key becomes the current key, the signed update revokes the compromised key, and the
/// re-sync chain drops it. The action writes its audit records.
#[tokio::test]
async fn adr_0030_a_compromise_rotates_and_retires_the_key_with_the_reason() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = JobSigningService::new(provider());
    service
        .ensure_initialized(db.database())
        .await
        .expect("cannot create the key set");
    let before = service
        .load(db.database())
        .await
        .expect("cannot load")
        .expect("present");
    let compromised = before.current_key_id().to_owned();
    let next = before.next_key_id().to_owned();
    let (_, compromised_key) = public_key(db.database(), SigningKeyRole::Current).await;
    let mut ring = KeyRing::new();
    ring.insert(compromised.clone(), compromised_key)
        .expect("the ring takes the key");
    ring.apply_key_update(&before.key_update(OffsetDateTime::now_utc()))
        .expect("the agent holds the next key");
    let forged_by_holder = before.sign_job(&job());
    assert_eq!(verify_job(&forged_by_holder, &ring, &expected()), Ok(job()));

    let retired = service
        .mark_compromised(db.database(), OffsetDateTime::now_utc(), &event())
        .await
        .expect("the compromise must succeed");
    assert_eq!(retired, vec![compromised.parse::<Uuid>().expect("an id")]);

    let after = service
        .load(db.database())
        .await
        .expect("cannot load")
        .expect("present");
    assert_eq!(after.current_key_id(), next);
    assert!(after.revoked_key_ids().contains(&compromised));
    let mut tx = db
        .database()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin");
    let rows = signing::list(&mut tx).await.expect("cannot list");
    let row = rows
        .iter()
        .find(|row| row.id.to_string() == compromised)
        .expect("the row stays");
    assert_eq!(row.retired_reason.as_deref(), Some("compromise"));
    assert!(row.retired_at.is_some());
    drop(tx);

    // The step that the control plane delivers is signed by the next key and names the revoked key.
    let step = service
        .current_step(db.database())
        .await
        .expect("the current step");
    assert_eq!(step.signer_key_id, next);
    ring.apply_key_update(&step)
        .expect("the agent takes the step");
    assert_eq!(
        ring.get(&compromised),
        None,
        "the key is revoked in the ring"
    );
    assert!(
        verify_job(&forged_by_holder, &ring, &expected()).is_err(),
        "a job that the compromised key signed is refused"
    );
    let signed = after.sign_job(&job());
    assert_eq!(signed.signer_key_id, next);
    assert_eq!(verify_job(&signed, &ring, &expected()), Ok(job()));

    let chain = service.key_chain(db.database()).await.expect("the chain");
    assert!(
        chain.iter().all(|step| step.signer_key_id != compromised),
        "no step that the compromised key signed"
    );
    assert_eq!(audit_count(&db, "signing_key_compromise").await, 1);
    assert_eq!(audit_count(&db, "signing_key_retire").await, 1);
    db.cleanup().await;
}

/// ADR-0030: only a Platform Admin runs the action; a tenant Admin gets HTTP 403.
#[tokio::test]
async fn adr_0030_the_compromise_endpoint_needs_the_platform_admin() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let uri = "/api/v1/platform/signing-keys/compromise";

    let (status, _, _) = common::send(
        &env.app,
        common::bearer_request("POST", uri, &env.admin, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(audit_count(&env.db, "signing_key_compromise").await, 0);

    let cookie = common::seed_session(
        env.db.database(),
        PLATFORM_TENANT_ID,
        "root",
        "platform_admin",
    )
    .await;
    let (status, _, body) =
        common::send(&env.app, common::request("POST", uri, Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["retired_key_ids"].as_array().map(Vec::len), Some(1));
    assert_eq!(audit_count(&env.db, "signing_key_compromise").await, 1);
    env.db.cleanup().await;
}

/// ADR-0030: the compromise reaches the agents of another control plane instance within the sweep
/// interval, so no instance keeps trusting the compromised key until a reconnect.
#[tokio::test]
async fn adr_0030_a_compromise_on_one_instance_reaches_the_agents_of_another() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let other = env.start_instance().await;
    let sweep = restichub_cp::agent_stream::spawn_revocation_sweep(
        other.state.clone(),
        REVOCATION_SWEEP_INTERVAL,
    );
    let enrolled = env.enroll_agent("compromise-host").await;
    let mut agent = env.connect_to(&other, enrolled).await;
    let signing_other = other.state.job_signing().expect("the signing service");
    signing_other.announce_remote_change(other.state.db()).await;

    let compromised = signing_other
        .current_key_id(other.state.db())
        .await
        .expect("a current key");
    let retired = env
        .state
        .job_signing()
        .expect("the signing service")
        .mark_compromised(env.db.database(), OffsetDateTime::now_utc(), &event())
        .await
        .expect("the compromise must succeed");
    assert_eq!(retired.len(), 1);

    let revoked = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let message = next_message(&mut agent.inbound).await;
            if let Some(control_message::Payload::SigningKeyUpdate(update)) = message.payload {
                let inner = prost::Message::decode(update.key_update.as_slice())
                    .map(|inner: restichub_proto::agent::v1::KeyUpdate| inner)
                    .expect("a key update");
                if inner.revoked_key_ids.contains(&compromised) {
                    return true;
                }
            }
        }
    })
    .await
    .expect("the update must reach the other instance within the sweep interval");
    assert!(revoked);
    sweep.abort();
    env.db.cleanup().await;
}
