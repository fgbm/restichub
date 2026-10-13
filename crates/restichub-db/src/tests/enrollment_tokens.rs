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
