//! The job signing key management and the signing path (ADR-0026, FR-PR-03, FR-PR-11).
//!
//! The tests cover the key round trip, the database dump (AC-PR-02), the replaced key (AC-PR-02),
//! the wrong KEK, the rotation with the revoked key identifiers (AC-PR-09 supports) and the row
//! swap (T19).

#![allow(clippy::expect_used)]

mod pki_support;

use ed25519_dalek::{SigningKey, VerifyingKey};
use prost::Message;
use prost_types::Timestamp;
use restichub_canary_scan::{Canary, Scanner};
use restichub_cp::clock::ManualClock;
use restichub_cp::job_signing::{JobSigningError, JobSigningService, ROTATION_OVERLAP};
use restichub_cp::maintenance;
use restichub_db::audit::{self, AuditAction, AuditFilter, AuditPage};
use restichub_db::signing::{self, SigningKeyRow};
use restichub_db::{Database, PLATFORM_TENANT_ID, SigningKeyRole};
use restichub_proto::agent::v1::{Job, KeyUpdate, RejectReason};
use restichub_proto::signing::{Expected, KeyRing, Rejection, verify_job};
use restichub_secrets::{CryptoError, KekId, KekProvider, WrappedSigningKey, unwrap_signing_key};
use time::OffsetDateTime;
use uuid::Uuid;

use pki_support::{base64_encode, dump_all, hex, other_kek, provider, test_db};

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

fn wrapped(row: &SigningKeyRow) -> WrappedSigningKey {
    WrappedSigningKey {
        kek_id: KekId::parse(&row.kek_id).expect("the stored key ID is valid"),
        nonce: row
            .nonce
            .as_slice()
            .try_into()
            .expect("the nonce is 96 bits"),
        ciphertext: row.wrapped_key.clone(),
    }
}

/// Returns the seed of each stored key with its identifier.
async fn active_keys(db: &Database, kek: &dyn KekProvider) -> Vec<(String, [u8; 32])> {
    key_rows(db)
        .await
        .into_iter()
        .filter(|row| row.retired_at.is_none() && row.retire_after.is_none())
        .map(|row| {
            let key = unwrap_signing_key(
                kek,
                PLATFORM_TENANT_ID.as_uuid(),
                row.id,
                row.role.as_str(),
                &wrapped(&row),
            )
            .expect("the KEK must open the stored key");
            let seed: [u8; 32] = key.as_slice().try_into().expect("the key is 32 bytes");
            (row.id.to_string(), seed)
        })
        .collect()
}

async fn active_key(
    db: &Database,
    kek: &dyn KekProvider,
    role: SigningKeyRole,
) -> (String, [u8; 32]) {
    key_rows(db)
        .await
        .into_iter()
        .find(|row| row.role == role && row.retired_at.is_none() && row.retire_after.is_none())
        .map(|row| {
            let key = unwrap_signing_key(
                kek,
                PLATFORM_TENANT_ID.as_uuid(),
                row.id,
                row.role.as_str(),
                &wrapped(&row),
            )
            .expect("the KEK must open the stored key");
            let seed: [u8; 32] = key.as_slice().try_into().expect("the key is 32 bytes");
            (row.id.to_string(), seed)
        })
        .expect("the active key is present")
}

fn verifying(seed: &[u8; 32]) -> VerifyingKey {
    SigningKey::from_bytes(seed).verifying_key()
}

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

/// Swaps the wrapped key of two `signing_keys` rows in the owner connection.
async fn swap_wrapped_keys(
    db: &restichub_db::test_util::TestDatabase,
    first: String,
    second: String,
) {
    let first: uuid::Uuid = first.parse().expect("the key ID is a UUID");
    let second: uuid::Uuid = second.parse().expect("the key ID is a UUID");
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    // The FROM subquery reads both rows before the update, so the two blobs exchange in one
    // statement.
    sqlx::query(
        "UPDATE signing_keys AS s SET \
             wrapped_key = CASE WHEN s.id = $1 THEN t.second_key ELSE t.first_key END, \
             nonce = CASE WHEN s.id = $1 THEN t.second_nonce ELSE t.first_nonce END, \
             kek_id = CASE WHEN s.id = $1 THEN t.second_kek ELSE t.first_kek END \
         FROM ( \
             SELECT \
                 (SELECT wrapped_key FROM signing_keys WHERE id = $1) AS first_key, \
                 (SELECT nonce FROM signing_keys WHERE id = $1) AS first_nonce, \
                 (SELECT kek_id FROM signing_keys WHERE id = $1) AS first_kek, \
                 (SELECT wrapped_key FROM signing_keys WHERE id = $2) AS second_key, \
                 (SELECT nonce FROM signing_keys WHERE id = $2) AS second_nonce, \
                 (SELECT kek_id FROM signing_keys WHERE id = $2) AS second_kek \
         ) AS t \
         WHERE s.id IN ($1, $2)",
    )
    .bind(first)
    .bind(second)
    .execute(&mut *tx)
    .await
    .expect("cannot swap the row");
    tx.commit().await.expect("cannot commit the swap");
}

/// Sets `retire_after` on one row in the owner connection, to build an abnormal overlap state.
async fn set_retire_after(
    db: &restichub_db::test_util::TestDatabase,
    id: Uuid,
    when: OffsetDateTime,
) {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query("UPDATE signing_keys SET retire_after = $1 WHERE id = $2")
        .bind(when)
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect("cannot schedule the retirement");
    tx.commit().await.expect("cannot commit the retirement");
}

/// Retires every row in the owner connection, to build the all-retired state.
async fn retire_all_rows(db: &restichub_db::test_util::TestDatabase) {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(
        "UPDATE signing_keys SET retired_at = now(), retire_after = now() \
         WHERE retired_at IS NULL",
    )
    .execute(&mut *tx)
    .await
    .expect("cannot retire the rows");
    tx.commit().await.expect("cannot commit the retirement");
}

/// The identifier of the one row that is scheduled for retirement and not yet retired.
async fn pending_key(db: &Database) -> Uuid {
    key_rows(db)
        .await
        .into_iter()
        .find(|row| row.retire_after.is_some() && row.retired_at.is_none())
        .map(|row| row.id)
        .expect("a pending key is present")
}

/// Overwrites `public_key` of one row in the owner connection, to break the row binding.
async fn overwrite_public_key(db: &restichub_db::test_util::TestDatabase, id: Uuid, bytes: &[u8]) {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query("UPDATE signing_keys SET public_key = $1 WHERE id = $2")
        .bind(bytes)
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect("cannot overwrite the public key");
    tx.commit().await.expect("cannot commit the overwrite");
}

#[tokio::test]
async fn a_signing_key_set_round_trips() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;

    // The second start verifies the stored keys and the KEK.
    let set = service
        .ensure_initialized(db.database())
        .await
        .expect("the second start must verify the keys");
    assert_ne!(set.current_key_id(), set.next_key_id());

    let keys = active_keys(db.database(), provider().as_ref()).await;
    assert_eq!(keys.len(), 2, "a current and a next key");

    db.cleanup().await;
}

#[tokio::test]
async fn the_signing_path_signs_a_job_that_the_agent_verifies() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let set = service
        .load(db.database())
        .await
        .expect("cannot load the key set")
        .expect("the key set is present");
    let (current_id, current_seed) =
        active_key(db.database(), provider().as_ref(), SigningKeyRole::Current).await;

    let signed = set.sign_job(&job());
    assert_eq!(signed.signer_key_id, current_id);

    let mut ring = KeyRing::new();
    assert!(ring.insert(current_id, verifying(&current_seed)).is_ok());
    assert_eq!(verify_job(&signed, &ring, &expected()), Ok(job()));

    db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_02_a_database_dump_holds_no_job_signing_key() {
    let Some(db) = test_db().await else {
        return;
    };
    create_set(db.database()).await;

    // The canary is the Ed25519 seed as hex and as base64. A `bytea` column renders as hex in the
    // dump, so the hex form catches a leaked key.
    let keys = active_keys(db.database(), provider().as_ref()).await;
    assert_eq!(keys.len(), 2, "a current and a next key");
    let mut canaries = Vec::new();
    for (_, seed) in &keys {
        canaries.push(Canary::parse(&hex(seed)));
        canaries.push(Canary::parse(&base64_encode(seed)));
    }

    let dump = dump_all(&db, PLATFORM_TENANT_ID).await;
    assert!(
        dump.contains("signing_keys"),
        "the dump must reach the signing_keys table"
    );
    let scanner = Scanner::new(&canaries);
    let findings = scanner.scan("dump", dump.as_bytes());
    assert!(
        findings.is_empty(),
        "the dump holds a job signing key: {findings:?}"
    );

    db.cleanup().await;
}

/// AC-PR-02: a replaced signing key makes the agent reject the new jobs.
#[tokio::test]
async fn ac_pr_02_a_replaced_key_makes_the_agent_reject_new_jobs() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let before = service
        .load(db.database())
        .await
        .expect("cannot load the key set")
        .expect("the key set is present");
    let (first_id, first_seed) =
        active_key(db.database(), provider().as_ref(), SigningKeyRole::Current).await;

    // One agent holds only the first key; another re-synced the delivered key set.
    let mut stale_ring = KeyRing::new();
    assert!(
        stale_ring
            .insert(first_id.clone(), verifying(&first_seed))
            .is_ok()
    );
    let mut synced_ring = KeyRing::new();
    assert!(synced_ring.insert(first_id, verifying(&first_seed)).is_ok());
    assert!(
        synced_ring
            .apply_key_update(&before.key_update(OffsetDateTime::now_utc()))
            .is_ok()
    );

    // The control plane replaces the key with a rotation.
    let set = service
        .rotate(db.database(), OffsetDateTime::now_utc())
        .await
        .expect("cannot rotate the key set");
    let signed = set.sign_job(&job());

    assert_eq!(
        verify_job(&signed, &stale_ring, &expected()),
        Err(Rejection {
            reason: RejectReason::UnknownKey,
            job_id: None,
        })
    );
    assert_eq!(verify_job(&signed, &synced_ring, &expected()), Ok(job()));

    db.cleanup().await;
}

/// AC-PR-09 supports: the control plane sends the key set with the new key ID and the revoked key
/// IDs.
#[tokio::test]
async fn ac_pr_09_a_rotation_sends_the_new_key_and_the_revoked_key_ids() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let before = service
        .load(db.database())
        .await
        .expect("cannot load the key set")
        .expect("the key set is present");
    let first_current = before.current_key_id().to_owned();
    let first_next = before.next_key_id().to_owned();
    let (_, first_seed) =
        active_key(db.database(), provider().as_ref(), SigningKeyRole::Current).await;

    // The delivery before the rotation hands the agent the next key.
    let mut ring = KeyRing::new();
    assert!(
        ring.insert(first_current.clone(), verifying(&first_seed))
            .is_ok()
    );
    assert!(
        ring.apply_key_update(&before.key_update(OffsetDateTime::now_utc()))
            .is_ok()
    );

    let after = service
        .rotate(db.database(), OffsetDateTime::now_utc())
        .await
        .expect("cannot rotate the key set");
    assert_eq!(after.current_key_id(), first_next);
    assert_ne!(after.next_key_id(), first_next);

    // The promoted key signs the jobs at once; the agent already holds it.
    let signed = after.sign_job(&job());
    assert_eq!(signed.signer_key_id, first_next);
    assert_eq!(verify_job(&signed, &ring, &expected()), Ok(job()));

    // The agent that held the old current key acknowledged the new set, so the old key retires.
    let first_current_id: Uuid = first_current.parse().expect("the key ID is a UUID");
    let retired = service
        .retire_acknowledged(db.database(), first_current_id, OffsetDateTime::now_utc())
        .await
        .expect("cannot retire the old key");
    assert_eq!(retired, 1);

    let set = service
        .load(db.database())
        .await
        .expect("cannot load the key set")
        .expect("the key set is present");
    assert_eq!(set.revoked_key_ids(), std::slice::from_ref(&first_current));
    let update = set.key_update(OffsetDateTime::now_utc());
    assert_eq!(update.signer_key_id, first_next);
    assert!(set.revoked_key_ids().contains(&first_current));

    // A reconnecting agent re-syncs the set and learns the revocation.
    let mut reconnected = ring.clone();
    assert!(
        reconnected
            .apply_key_update(&set.key_update(OffsetDateTime::now_utc()))
            .is_ok()
    );
    assert_eq!(reconnected.get(&first_current), None);
    assert_eq!(verify_job(&signed, &reconnected, &expected()), Ok(job()));

    db.cleanup().await;
}

#[tokio::test]
async fn retire_due_respects_the_overlap_window() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let now = OffsetDateTime::now_utc();
    service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate the key set");

    // Inside the window the old key stays valid.
    assert_eq!(
        service
            .retire_due(db.database(), now)
            .await
            .expect("cannot retire"),
        0
    );
    // At most seven days after the rotation starts the old key retires.
    assert_eq!(
        service
            .retire_due(
                db.database(),
                now + ROTATION_OVERLAP + time::Duration::seconds(1)
            )
            .await
            .expect("cannot retire"),
        1
    );

    db.cleanup().await;
}

#[tokio::test]
async fn t19_a_swapped_signing_key_blob_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let (current, _) =
        active_key(db.database(), provider().as_ref(), SigningKeyRole::Current).await;
    let (next, _) = active_key(db.database(), provider().as_ref(), SigningKeyRole::Next).await;

    // The associated data binds a blob to its role, so a current/next swap fails the check (T19).
    swap_wrapped_keys(&db, current, next).await;

    let error = service.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(JobSigningError::Crypto(CryptoError::Decrypt(_)))),
        "{error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_wrong_kek_refuses_the_signing_keys() {
    let Some(db) = test_db().await else {
        return;
    };
    create_set(db.database()).await;

    // A provider that holds another key cannot unwrap the stored signing keys.
    let other = JobSigningService::new(other_kek());
    let error = other.ensure_initialized(db.database()).await;
    assert!(
        matches!(
            error,
            Err(JobSigningError::Crypto(CryptoError::UnknownKek(_)))
        ),
        "{error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn two_concurrent_first_starts_create_one_key_set() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = JobSigningService::new(provider());
    // The advisory lock serializes the two starts, so exactly one creates the set.
    let (first, second) = tokio::join!(
        service.ensure_initialized(db.database()),
        service.ensure_initialized(db.database())
    );
    assert!(first.is_ok(), "{first:?}");
    assert!(second.is_ok(), "{second:?}");

    let rows = key_rows(db.database()).await;
    assert_eq!(
        rows.iter()
            .filter(|row| row.role == SigningKeyRole::Current && row.retired_at.is_none())
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.role == SigningKeyRole::Next && row.retired_at.is_none())
            .count(),
        1
    );

    db.cleanup().await;
}

/// Item 1 (#49): the periodic control plane task retires the old key after the overlap, so the
/// seven-day bound holds without a manual call. The clock is injectable, so no test sleeps.
#[tokio::test]
async fn the_periodic_retirement_retires_the_old_key_after_the_overlap() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let before = service
        .load(db.database())
        .await
        .expect("cannot load the key set")
        .expect("the key set is present");
    let old_current = before.current_key_id().to_owned();

    let now = OffsetDateTime::now_utc();
    service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate the key set");

    // The clock advances past the overlap; the periodic step reads it and retires the old key.
    let clock = ManualClock::new(now);
    clock.advance((ROTATION_OVERLAP + time::Duration::seconds(1)).whole_seconds());
    maintenance::retire_signing_keys(db.database(), &service, &clock)
        .await
        .expect("cannot retire the old key");

    let set = service
        .load(db.database())
        .await
        .expect("cannot load the key set")
        .expect("the key set is present");
    assert!(set.revoked_key_ids().contains(&old_current));

    let update = set.key_update(OffsetDateTime::now_utc());
    let decoded =
        KeyUpdate::decode(update.key_update.as_slice()).expect("the key update must decode");
    assert!(decoded.revoked_key_ids.contains(&old_current));

    db.cleanup().await;
}

/// Item 3 (#49): a rotation refuses while an earlier rotation still holds its overlap.
#[tokio::test]
async fn a_rotation_with_an_open_overlap_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let now = OffsetDateTime::now_utc();
    service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate the key set");

    let second = service.rotate(db.database(), now).await;
    assert!(
        matches!(second, Err(JobSigningError::RotationPending)),
        "{second:?}"
    );

    // Once the pending key is acknowledged, the next rotation is allowed.
    let pending = pending_key(db.database()).await;
    service
        .retire_acknowledged(db.database(), pending, now)
        .await
        .expect("cannot retire the pending key");
    assert!(
        service.rotate(db.database(), now).await.is_ok(),
        "the rotation must be allowed after the acknowledgement"
    );

    db.cleanup().await;
}

/// Item 3 (#49): `retire_acknowledged` retires only the named key and leaves a newer rotation's
/// key alone.
#[tokio::test]
async fn retire_acknowledged_retires_only_the_named_key() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let now = OffsetDateTime::now_utc();
    let rotated = service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate the key set");
    let older = pending_key(db.database()).await;
    let newer: Uuid = rotated
        .current_key_id()
        .parse()
        .expect("the key ID is a UUID");
    // A second pending row is an abnormal state; the method must still touch only the named key.
    set_retire_after(&db, newer, now + time::Duration::days(1)).await;

    let retired = service
        .retire_acknowledged(db.database(), older, now)
        .await
        .expect("cannot retire the old key");
    assert_eq!(retired, 1);

    let rows = key_rows(db.database()).await;
    let newer_row = rows
        .iter()
        .find(|row| row.id == newer)
        .expect("the newer row is present");
    assert!(
        newer_row.retired_at.is_none(),
        "the newer rotation's key must stay pending"
    );
    let older_row = rows
        .iter()
        .find(|row| row.id == older)
        .expect("the older row is present");
    assert!(older_row.retired_at.is_some());

    db.cleanup().await;
}

/// Item 4 (#49): the start creates a key set only when the table is empty; every row retired
/// refuses the start.
#[tokio::test]
async fn every_retired_key_set_refuses_the_start() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    retire_all_rows(&db).await;

    let error = service.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(JobSigningError::AllRetired)),
        "{error:?}"
    );

    db.cleanup().await;
}

/// Item 2 (T19): a blob swapped between the retired current key and the active current key does
/// not unwrap, because the associated data binds the key ID and the public key binds the row.
#[tokio::test]
async fn t19_a_swapped_retired_and_active_blob_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let (first_current, _) =
        active_key(db.database(), provider().as_ref(), SigningKeyRole::Current).await;
    let now = OffsetDateTime::now_utc();
    let rotated = service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate the key set");
    let active_current = rotated.current_key_id().to_owned();
    service
        .retire_acknowledged(
            db.database(),
            first_current.parse().expect("the key ID is a UUID"),
            now,
        )
        .await
        .expect("cannot retire the old key");

    swap_wrapped_keys(&db, first_current, active_current).await;

    let error = service.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(JobSigningError::Crypto(CryptoError::Decrypt(_)))),
        "{error:?}"
    );

    db.cleanup().await;
}

/// Item 7 (#49): a rotation on one control plane is visible to another signing service, which
/// reloads the key set on the changed version and signs with the new current key.
#[tokio::test]
async fn a_rotation_is_visible_to_another_signing_service() {
    let Some(db) = test_db().await else {
        return;
    };
    let first = create_set(db.database()).await;
    let second = JobSigningService::new(provider());

    let before = second
        .sign_job(db.database(), &job())
        .await
        .expect("cannot sign the job");
    let before_current = before.signer_key_id.clone();

    let rotated = first
        .rotate(db.database(), OffsetDateTime::now_utc())
        .await
        .expect("cannot rotate the key set");
    assert_ne!(rotated.current_key_id(), before_current);

    let after = second
        .sign_job(db.database(), &job())
        .await
        .expect("cannot sign the job");
    assert_eq!(after.signer_key_id, rotated.current_key_id());

    db.cleanup().await;
}

/// Item 1 of the second review: the version token notices a retirement, so the service rebuilds
/// the key update and lists the newly revoked key.
#[tokio::test]
async fn a_retirement_is_visible_to_the_signing_service() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let before = service
        .load(db.database())
        .await
        .expect("cannot load the key set")
        .expect("the key set is present");
    let old_current = before.current_key_id().to_owned();
    let now = OffsetDateTime::now_utc();
    service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate the key set");

    // The first update fills the cache; the old key is not revoked yet.
    let cached = service
        .key_update(db.database(), now)
        .await
        .expect("cannot build the key update");
    let cached = KeyUpdate::decode(cached.key_update.as_slice()).expect("the update must decode");
    assert!(!cached.revoked_key_ids.contains(&old_current));

    service
        .retire_acknowledged(
            db.database(),
            old_current.parse().expect("the key ID is a UUID"),
            now,
        )
        .await
        .expect("cannot retire the old key");

    let update = service
        .key_update(db.database(), now)
        .await
        .expect("cannot build the key update");
    let decoded = KeyUpdate::decode(update.key_update.as_slice()).expect("the update must decode");
    assert!(decoded.revoked_key_ids.contains(&old_current));

    db.cleanup().await;
}

/// Item 3 of the second review: a row whose `public_key` does not match the unwrapped key is
/// refused with `KeyMismatch`.
#[tokio::test]
async fn a_wrong_public_key_refuses_the_key() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let (current, _) =
        active_key(db.database(), provider().as_ref(), SigningKeyRole::Current).await;
    overwrite_public_key(
        &db,
        current.parse().expect("the key ID is a UUID"),
        &[9_u8; 32],
    )
    .await;

    let error = service.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(JobSigningError::KeyMismatch { .. })),
        "{error:?}"
    );

    db.cleanup().await;
}

/// Item 5 of the second review: a retirement writes a `signing_key_retire` audit record in the
/// same transaction.
#[tokio::test]
async fn a_retirement_writes_an_audit_record() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_set(db.database()).await;
    let now = OffsetDateTime::now_utc();
    service
        .rotate(db.database(), now)
        .await
        .expect("cannot rotate the key set");
    let pending = pending_key(db.database()).await;
    let retired = service
        .retire_acknowledged(db.database(), pending, now)
        .await
        .expect("cannot retire the old key");
    assert_eq!(retired, 1);

    let mut tx = db
        .database()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let filter = AuditFilter {
        action: Some(AuditAction::SigningKeyRetire),
        ..AuditFilter::default()
    };
    let result = audit::list(&mut tx, &filter, &AuditPage::default())
        .await
        .expect("cannot list the audit log");
    tx.commit().await.expect("cannot commit the read");
    assert_eq!(result.records.len(), 1, "one retirement record");
    assert_eq!(result.records[0].object_id, Some(pending));

    db.cleanup().await;
}
