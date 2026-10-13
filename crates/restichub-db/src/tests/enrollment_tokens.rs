//! The enrollment token lifecycle at the database layer (AC-ID-01).

use time::Duration;
use time::OffsetDateTime;

use super::*;
use crate::enrollment_tokens::{self, ConsumeOutcome, NewEnrollmentToken};
use crate::test_util::test_database;

async fn tenant_with_admin(
    db: &Database,
    name: &str,
) -> Result<(TenantId, UserId), Box<dyn std::error::Error>> {
    let mut tx = db.begin_platform().await?;
    let tenant = crate::tenants::create(&mut tx, name).await?;
    tx.commit().await?;
    let mut tx = db.begin_tenant(tenant).await?;
    let username = format!("admin-{tenant}");
    let user = users::create_member(
        &mut tx,
        &NewUser {
            username: &username,
            password_hash: "phc",
            display_name: "Admin",
        },
        crate::Role::Admin,
    )
    .await?;
    tx.commit().await?;
    Ok((tenant, user))
}

#[tokio::test]
async fn ac_id_01_a_used_or_expired_token_is_refused() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, user) = tenant_with_admin(db.database(), "enroll-a").await?;
    let now = OffsetDateTime::now_utc();
    let valid = [1u8; 32];
    let expired = [2u8; 32];

    let mut tx = db.database().begin_tenant(tenant).await?;
    let labels = ["host".to_owned()];
    enrollment_tokens::create(
        &mut tx,
        &NewEnrollmentToken {
            token_hash: &valid,
            display_prefix: "ab12cd34",
            labels: &labels,
            expires_at: now + Duration::minutes(15),
            created_by: user,
        },
    )
    .await?;
    enrollment_tokens::create(
        &mut tx,
        &NewEnrollmentToken {
            token_hash: &expired,
            display_prefix: "ef56gh78",
            labels: &labels,
            expires_at: now - Duration::minutes(1),
            created_by: user,
        },
    )
    .await?;

    // Spent once, then refused.
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &valid, now).await?,
        ConsumeOutcome::Consumed
    );
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &valid, now).await?,
        ConsumeOutcome::AlreadyUsed
    );
    // An expired token is refused with its own outcome.
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &expired, now).await?,
        ConsumeOutcome::Expired
    );
    // Unknown hashes are refused.
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &[9u8; 32], now).await?,
        ConsumeOutcome::Unknown
    );
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_id_01_a_token_of_another_tenant_is_refused() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _user_a) = tenant_with_admin(db.database(), "enroll-a").await?;
    let (tenant_b, user_b) = tenant_with_admin(db.database(), "enroll-b").await?;
    let now = OffsetDateTime::now_utc();
    let labels = ["host".to_owned()];

    // Tenant B holds a valid token.
    let mut tx = db.database().begin_tenant(tenant_b).await?;
    enrollment_tokens::create(
        &mut tx,
        &NewEnrollmentToken {
            token_hash: &[3u8; 32],
            display_prefix: "ab12cd34",
            labels: &labels,
            expires_at: now + Duration::minutes(15),
            created_by: user_b,
        },
    )
    .await?;
    tx.commit().await?;

    // Tenant A sees no token: the hash of tenant B is unknown and the row count is zero.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &[3u8; 32], now).await?,
        ConsumeOutcome::Unknown
    );
    let page = enrollment_tokens::list(&mut tx, &Default::default()).await?;
    assert!(page.tokens.is_empty());
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let page = enrollment_tokens::list(&mut tx, &Default::default()).await?;
    assert_eq!(page.tokens.len(), 1);
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_id_01_a_revoked_token_is_refused_and_a_second_revoke_changes_nothing() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, user) = tenant_with_admin(db.database(), "enroll-revoke").await?;
    let now = OffsetDateTime::now_utc();
    let hash = [4u8; 32];

    let id = {
        let mut tx = db.database().begin_tenant(tenant).await?;
        let id = enrollment_tokens::create(
            &mut tx,
            &NewEnrollmentToken {
                token_hash: &hash,
                display_prefix: "ab12cd34",
                labels: &["host".to_owned()],
                expires_at: now + Duration::hours(1),
                created_by: user,
            },
        )
        .await?;
        tx.commit().await?;
        id
    };

    // The revoke is soft: the row stays and the audit trail keeps the actor.
    let mut tx = db.database().begin_tenant(tenant).await?;
    assert!(enrollment_tokens::revoke(&mut tx, id, user, now).await?);
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &hash, now).await?,
        ConsumeOutcome::Revoked
    );
    let token = enrollment_tokens::get(&mut tx, id)
        .await?
        .expect("the revoked token stays");
    assert!(token.revoked_at.is_some());
    assert_eq!(token.revoked_by_user_id, Some(user));
    // Second revoke: no change.
    assert!(!enrollment_tokens::revoke(&mut tx, id, user, now).await?);
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

/// A concurrent revoke and exchange: one of the two wins, and only one.
#[tokio::test]
async fn ac_id_01_a_revoke_and_a_consume_race_resolves_to_one_winner() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, user) = tenant_with_admin(db.database(), "enroll-race").await?;
    let now = OffsetDateTime::now_utc();
    let hash = [5u8; 32];
    let id = {
        let mut tx = db.database().begin_tenant(tenant).await?;
        let id = enrollment_tokens::create(
            &mut tx,
            &NewEnrollmentToken {
                token_hash: &hash,
                display_prefix: "ab12cd34",
                labels: &["host".to_owned()],
                expires_at: now + Duration::hours(1),
                created_by: user,
            },
        )
        .await?;
        tx.commit().await?;
        id
    };

    let database = db.database();
    let revoke_branch = async {
        let mut tx = database.begin_tenant(tenant).await?;
        let changed = enrollment_tokens::revoke(&mut tx, id, user, now).await?;
        tx.commit().await?;
        Ok::<bool, crate::DbError>(changed)
    };
    let consume_branch = async {
        let mut tx = database.begin_tenant(tenant).await?;
        let outcome = enrollment_tokens::consume(&mut tx, &hash, now).await?;
        tx.commit().await?;
        Ok::<ConsumeOutcome, crate::DbError>(outcome)
    };
    let (revoked, consumed) = tokio::join!(revoke_branch, consume_branch);
    let revoked = revoked?;
    let consumed = consumed?;

    match (revoked, consumed) {
        (true, ConsumeOutcome::Revoked) => {}
        (false, ConsumeOutcome::Consumed) => {}
        other => panic!("the race must have one winner: {other:?}"),
    }

    db.cleanup().await;
    Ok(())
}
