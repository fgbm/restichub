//! The re-sync chain of the job signing keys (ADR-0030, FR-PR-11).
//!
//! The chain delivers the stored steps of the rotations. A key that is still in its overlap stays
//! trusted, a key retired for a compromise never enters the chain, and one step that does not
//! decode never blocks the current step.

#![allow(clippy::expect_used)]

mod pki_support;

use prost::Message as _;
use restichub_cp::job_signing::{JobSigningService, MAX_CHAIN_STEPS};
use restichub_db::signing::{self, SigningKeyRow};
use restichub_db::{Database, PLATFORM_TENANT_ID};
use restichub_proto::agent::v1::KeyUpdate;
use restichub_proto::agent::v1::SigningKeyUpdate;
use time::OffsetDateTime;
use uuid::Uuid;

use pki_support::{provider, test_db};

async fn create_set(db: &Database) -> JobSigningService {
    let service = JobSigningService::new(provider());
    service
        .ensure_initialized(db)
        .await
        .expect("cannot create the key set");
    service
}

async fn key_rows(db: &Database) -> Vec<SigningKeyRow> {
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let rows = signing::list(&mut tx)
        .await
        .expect("cannot read the key rows");
    tx.commit().await.expect("cannot commit the read");
    rows
}

fn inner(update: &SigningKeyUpdate) -> KeyUpdate {
    KeyUpdate::decode(update.key_update.as_slice()).expect("the step decodes")
}

/// FR-PR-11: the chain does not revoke a key that is still in its rotation overlap.
#[tokio::test]
async fn m2_a_key_in_overlap_stays_trusted() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let before = service
        .load(db.database())
        .await
        .expect("cannot load the set")
        .expect("the set is present");
    let old_current = before.current_key_id().to_owned();

    service
        .rotate(db.database(), OffsetDateTime::now_utc())
        .await
        .expect("cannot rotate");

    let chain = service
        .key_chain(db.database())
        .await
        .expect("cannot build the chain");
    let last = chain.last().expect("the chain holds the current step");
    assert!(
        !inner(last).revoked_key_ids.contains(&old_current),
        "a key in its overlap must not be revoked"
    );

    db.cleanup().await;
}

/// ADR-0030: the chain is bounded to the newest steps and still ends on the current step.
#[tokio::test]
async fn m2_the_chain_keeps_the_newest_steps() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let mut now = OffsetDateTime::now_utc();
    // Each rotation retires the previous overlap, so the next rotation is allowed.
    for _ in 0..(MAX_CHAIN_STEPS + 6) {
        service
            .rotate(db.database(), now)
            .await
            .expect("cannot rotate");
        now += time::Duration::days(8);
        service
            .retire_due(db.database(), now)
            .await
            .expect("cannot retire");
    }
    let current_next = service
        .load(db.database())
        .await
        .expect("cannot load the set")
        .expect("the set is present")
        .next_key_id()
        .to_owned();

    let chain = service
        .key_chain(db.database())
        .await
        .expect("cannot build the chain");
    assert_eq!(chain.len(), MAX_CHAIN_STEPS);
    let last = chain.last().expect("the chain is not empty");
    assert_eq!(
        inner(last).key_id,
        current_next,
        "the last step must introduce the current next key"
    );

    db.cleanup().await;
}

/// ADR-0030: a step that does not decode is skipped and the current step is still delivered.
#[tokio::test]
async fn m1_a_corrupt_step_does_not_block_the_current_step() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let mut now = OffsetDateTime::now_utc();
    // Two rotations: the first retires the initial current key, which holds no step, and the second
    // retires a key that a rotation introduced and that therefore carries a step.
    for _ in 0..2 {
        service
            .rotate(db.database(), now)
            .await
            .expect("cannot rotate");
        now += time::Duration::days(8);
        service
            .retire_due(db.database(), now)
            .await
            .expect("cannot retire");
    }

    // Corrupt the step of the oldest row that carries one, so the chain skips it.
    let rows = key_rows(db.database()).await;
    let victim = rows
        .iter()
        .find(|row| row.step_update.is_some() && row.retired_at.is_some())
        .expect("a retired row holds a step");
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query("UPDATE signing_keys SET step_update = $1 WHERE id = $2")
        .bind(vec![0xff_u8, 0xff, 0xff])
        .bind(victim.id)
        .execute(&mut *tx)
        .await
        .expect("cannot corrupt the step");
    tx.commit().await.expect("cannot commit the corrupt step");

    let current_next = service
        .load(db.database())
        .await
        .expect("cannot load the set")
        .expect("the set is present")
        .next_key_id()
        .to_owned();
    let chain = service
        .key_chain(db.database())
        .await
        .expect("cannot build the chain");
    let last = chain.last().expect("the chain holds the current step");
    assert_eq!(inner(last).key_id, current_next);

    db.cleanup().await;
}

/// ADR-0030: a key retired for a compromise never enters the chain.
#[tokio::test]
async fn a_compromise_key_is_not_in_the_chain() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let mut now = OffsetDateTime::now_utc();
    service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate");
    now += time::Duration::days(8);
    service
        .retire_due(db.database(), now)
        .await
        .expect("cannot retire");
    service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate");

    // The current key introduced the newest step and signed the step after it, so a compromise
    // removes two steps and the steps of the earlier rotations stay.
    let victim = Uuid::parse_str(
        service
            .load(db.database())
            .await
            .expect("cannot load the set")
            .expect("the set is present")
            .current_key_id(),
    )
    .expect("the key id is a uuid");

    let mut tx = db
        .database()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let retired = signing::retire_compromised(&mut tx, victim, OffsetDateTime::now_utc())
        .await
        .expect("cannot retire the key");
    tx.commit().await.expect("cannot commit the retirement");
    assert_eq!(retired, vec![victim]);

    let chain = service
        .key_chain(db.database())
        .await
        .expect("cannot build the chain");
    assert!(
        !chain.is_empty(),
        "the steps of the earlier rotations survive the compromise"
    );
    assert!(
        chain
            .iter()
            .all(|update| inner(update).key_id != victim.to_string()),
        "a compromise-retired key must not enter the chain"
    );

    db.cleanup().await;
}
