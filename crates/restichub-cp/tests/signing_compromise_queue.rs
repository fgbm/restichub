//! A stored signed job and the compromise of the job signing key (ADR-0030, ADR-0020, T13).
//!
//! A job in the status `sent` goes out again when its lease expires. The redelivery sends the
//! stored bytes, so the control plane must not send a blob that a compromised key signed.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_db::audit::{AuditAction, AuditEvent};
use time::OffsetDateTime;

use queue_support::{Env, decode_job_id, test_queue};

/// ADR-0030: after a compromise, the redelivery signs the stored job again with the current key.
/// The agent never gets the blob of the compromised key.
#[tokio::test]
async fn adr_0030_a_redelivery_after_a_compromise_never_sends_the_old_blob() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("compromise-queue-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, serde_json::json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let first = agent.job().await;
    assert_eq!(env.job_status(job).await, "sent");

    let signing = env.state.job_signing().expect("the signing service");
    let compromised = signing
        .current_key_id(env.db.database())
        .await
        .expect("a current key");
    assert_eq!(first.signer_key_id, compromised);
    let template = AuditEvent::new(AuditAction::SigningKeyCompromise, "signing_key");
    signing
        .mark_compromised(
            env.db.database(),
            OffsetDateTime::now_utc(),
            &compromised,
            &template,
        )
        .await
        .expect("the compromise must succeed");
    let current = signing
        .current_key_id(env.db.database())
        .await
        .expect("a current key");
    assert_ne!(current, compromised);

    env.clock.advance(61);
    env.pass().await;
    let second = agent
        .try_job(Duration::from_secs(2))
        .await
        .expect("the job goes out again");
    assert_ne!(
        second.signer_key_id, compromised,
        "the old blob must not be sent"
    );
    assert_eq!(second.signer_key_id, current);
    assert_eq!(
        decode_job_id(&second),
        decode_job_id(&first),
        "the same job"
    );
    assert_eq!(env.job_status(job).await, "sent");
    env.db.cleanup().await;
}
