use time::{Duration, OffsetDateTime};

use super::*;
use crate::restore::{self, JobRequest, LsRequest, SnapshotsRequest};
use crate::test_util::test_database;
use crate::{JobId, RepositoryId};

async fn repository_of(tx: &mut TenantTx<'_>) -> Result<RepositoryId, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM repositories LIMIT 1")
        .fetch_one(tx.as_conn())
        .await
}

async fn finished_job(
    tx: &mut TenantTx<'_>,
    tenant: TenantId,
    kind: &str,
    finished_at: OffsetDateTime,
) -> Result<JobId, sqlx::Error> {
    let id = JobId::new();
    sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, agent_id, repository_id, status, finished_at) \
         VALUES ($1, $2, $3, (SELECT id FROM agents WHERE tenant_id = $2), \
                 (SELECT id FROM repositories WHERE tenant_id = $2), 'succeeded', $4)",
    )
    .bind(id)
    .bind(tenant)
    .bind(kind)
    .bind(finished_at)
    .execute(tx.as_conn())
    .await?;
    Ok(id)
}

fn page(offset: u64) -> LsRequest {
    LsRequest {
        snapshot_id: "0123abcd".to_owned(),
        path: "/srv".to_owned(),
        offset,
        limit: 1_000,
    }
}

/// AC-RS-02: a page that a finished job listed answers the same request again, and only that.
#[tokio::test]
async fn ac_rs_02_db_a_stored_page_answers_the_same_request_only() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    let now = OffsetDateTime::now_utc();

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let repository = repository_of(&mut tx).await?;
    let job = finished_job(&mut tx, tenant_a, "ls", now).await?;
    restore::insert_request(&mut tx, job, &JobRequest::Ls(page(0))).await?;
    let payload = serde_json::json!({"last": true, "nodes": []});
    restore::insert_listing(&mut tx, job, "ls", &payload, now).await?;
    // The agent sends a step result twice, so a second write must keep the first row.
    restore::insert_listing(&mut tx, job, "ls", &serde_json::json!({"last": false}), now).await?;

    let hit = restore::cached_page(&mut tx, repository, &page(0), now - Duration::minutes(10))
        .await?
        .expect("the same request finds the page");
    assert_eq!(hit.job_id, job);
    assert_eq!(hit.payload, payload);
    assert!(
        restore::cached_page(
            &mut tx,
            repository,
            &page(1_000),
            now - Duration::minutes(10)
        )
        .await?
        .is_none(),
        "another offset is another page"
    );
    let mut other = page(0);
    other.path = "/etc".to_owned();
    assert!(
        restore::cached_page(&mut tx, repository, &other, now - Duration::minutes(10))
            .await?
            .is_none()
    );
    assert!(
        restore::cached_page(&mut tx, repository, &page(0), now + Duration::minutes(1))
            .await?
            .is_none(),
        "an old page is not served"
    );
    assert_eq!(
        restore::get_request(&mut tx, job).await?,
        Some(JobRequest::Ls(page(0)))
    );
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

/// FR-RS-01: the newest snapshot list of the repository wins, and the sweep removes old rows.
#[tokio::test]
async fn fr_rs_01_db_the_newest_snapshot_list_wins_and_the_sweep_removes_old_rows() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    let now = OffsetDateTime::now_utc();

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let repository = repository_of(&mut tx).await?;
    let old_time = now - Duration::days(8);
    let old = finished_job(&mut tx, tenant_a, "snapshots", old_time).await?;
    let new = finished_job(&mut tx, tenant_a, "snapshots", now).await?;
    let request = JobRequest::Snapshots(SnapshotsRequest::default());
    for (job, at) in [(old, old_time), (new, now)] {
        restore::insert_request(&mut tx, job, &request).await?;
        let payload = serde_json::json!({"truncated": false, "snapshots": []});
        restore::insert_listing(&mut tx, job, "snapshots", &payload, at).await?;
    }
    let latest = restore::latest_snapshots(&mut tx, repository)
        .await?
        .expect("a list");
    assert_eq!(latest.job_id, new);

    let removed = restore::delete_expired_listings(
        &mut tx,
        now - Duration::hours(1),
        now - Duration::days(7),
    )
    .await?;
    assert_eq!(removed, 1);
    assert!(restore::get_listing(&mut tx, old).await?.is_none());
    assert!(restore::get_listing(&mut tx, new).await?.is_some());

    let deleted = restore::delete_old_browse_jobs(&mut tx, now - Duration::days(7)).await?;
    assert_eq!(deleted, 1, "the old browse job goes with its request");
    assert!(restore::get_request(&mut tx, old).await?.is_none());
    assert!(restore::get_request(&mut tx, new).await?.is_some());
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

/// AC-TN-01: the tenant of a listing is the tenant of the transaction.
#[tokio::test]
async fn ac_tn_01_db_a_listing_of_another_tenant_is_invisible() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    seed_tenant(db.database(), tenant_b).await?;
    let now = OffsetDateTime::now_utc();

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let repository = repository_of(&mut tx).await?;
    let job = finished_job(&mut tx, tenant_a, "snapshots", now).await?;
    restore::insert_request(
        &mut tx,
        job,
        &JobRequest::Snapshots(SnapshotsRequest::default()),
    )
    .await?;
    restore::insert_listing(&mut tx, job, "snapshots", &serde_json::json!({}), now).await?;
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_b).await?;
    assert!(restore::get_listing(&mut tx, job).await?.is_none());
    assert!(restore::get_request(&mut tx, job).await?.is_none());
    assert!(
        restore::latest_snapshots(&mut tx, repository)
            .await?
            .is_none()
    );
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}
