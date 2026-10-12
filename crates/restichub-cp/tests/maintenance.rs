//! The periodic maintenance tests (issue #18).

mod common;

use restichub_cp::maintenance::cleanup;
use restichub_db::{PLATFORM_TENANT_ID, sessions, setup};
use time::OffsetDateTime;

use common::{count, create_tenant, create_user};

#[tokio::test]
async fn the_maintenance_removes_the_expired_rows() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "maintenance").await;
    let user = create_user(db.database(), tenant, "alice", "phc", "admin").await;
    let now = OffsetDateTime::now_utc();

    // An expired session and a live session.
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    sessions::create(
        &mut tx,
        user,
        &[1u8; 32],
        now - time::Duration::hours(1),
        now - time::Duration::hours(2),
        None,
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    sessions::create(
        &mut tx,
        user,
        &[2u8; 32],
        now + time::Duration::hours(1),
        now,
        None,
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    // An expired setup token and a used setup token.
    let mut tx = db
        .database()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    setup::create(&mut tx, &[3u8; 32], now - time::Duration::hours(1))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let used = setup::create(&mut tx, &[4u8; 32], now + time::Duration::hours(1))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    setup::mark_used(&mut tx, used, now)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    cleanup(db.database(), now)
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    // The live session stays and both setup tokens are removed.
    assert_eq!(count(db.database(), tenant, "sessions").await, 1);
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        0
    );

    db.cleanup().await;
}
