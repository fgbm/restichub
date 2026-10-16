use super::*;
use crate::alerts::{self, AlertKind, AlertParams, DeliveryEvent, DeliveryStatus, NewAlert};
use crate::channels::{self, ChannelKind, NewChannel};
use crate::enums::JobKind;
use crate::test_util::test_database;
use crate::{AlertId, ChannelId};
use time::{Duration, OffsetDateTime};

async fn email_channel(
    tx: &mut TenantTx<'_>,
    name: &str,
) -> Result<ChannelId, Box<dyn std::error::Error>> {
    let id = ChannelId::new();
    channels::create(
        tx,
        &NewChannel {
            id,
            name,
            kind: ChannelKind::Email,
            enabled: true,
            recipients: &["ops@example.com".to_owned()],
            url_origin: None,
            secret_id: None,
            url_secret_id: None,
            created_by: None,
        },
    )
    .await?;
    Ok(id)
}

async fn open_alert(
    tx: &mut TenantTx<'_>,
    key: &str,
    now: OffsetDateTime,
) -> Result<AlertId, crate::DbError> {
    let id = AlertId::new();
    let params = AlertParams::default();
    alerts::insert_open(
        tx,
        &NewAlert {
            id,
            kind: AlertKind::AgentOffline,
            dedupe_key: key,
            agent_id: None,
            plan_id: None,
            repository_id: None,
            job_id: None,
            agent_hostname: None,
            plan_name: None,
            repository_name: None,
            params: &params,
            detail: "",
        },
        now,
    )
    .await?;
    Ok(id)
}

/// AC-MN-07: the database allows one open alert for a kind and a key.
#[tokio::test]
async fn ac_mn_07_db_an_open_alert_is_unique_for_its_key() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    let first = open_alert(&mut tx, "agent:1", now).await?;
    let params = AlertParams::default();
    let second = alerts::insert_open(
        &mut tx,
        &NewAlert {
            id: AlertId::new(),
            kind: AlertKind::AgentOffline,
            dedupe_key: "agent:1",
            agent_id: None,
            plan_id: None,
            repository_id: None,
            job_id: None,
            agent_hostname: None,
            plan_name: None,
            repository_name: None,
            params: &params,
            detail: "",
        },
        now,
    )
    .await?;
    assert!(
        !second,
        "a second open alert with the same key changes nothing"
    );
    assert!(alerts::resolve(&mut tx, first, now).await?);
    assert!(
        !alerts::resolve(&mut tx, first, now).await?,
        "a resolved alert resolves once"
    );
    open_alert(&mut tx, "agent:1", now).await?;
    assert_eq!(alerts::open_alerts(&mut tx).await?.len(), 1);
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

/// AC-MN-07: a claim is exclusive and it is a lease.
#[tokio::test]
async fn ac_mn_07_db_claims_do_not_overlap_and_expire() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    email_channel(&mut tx, "mail").await?;
    for index in 0..3 {
        let alert = open_alert(&mut tx, &format!("agent:{index}"), now).await?;
        alerts::enqueue(&mut tx, alert, DeliveryEvent::Opened, now).await?;
    }
    tx.commit().await?;

    let lease = now + Duration::minutes(5);
    let mut first = db.database().begin_tenant(tenant).await?;
    let mut second = db.database().begin_tenant(tenant).await?;
    let claimed_first = alerts::claim_due(&mut first, now, lease, 2, 5).await?;
    let claimed_second = alerts::claim_due(&mut second, now, lease, 2, 5).await?;
    assert_eq!(claimed_first.len(), 2);
    assert_eq!(claimed_second.len(), 1, "the locked rows are skipped");
    for one in &claimed_first {
        assert!(claimed_second.iter().all(|other| other.id != one.id));
        assert_eq!(one.attempt, 0, "a claim counts no attempt");
    }
    first.commit().await?;
    second.commit().await?;

    let mut tx = db.database().begin_tenant(tenant).await?;
    assert!(
        alerts::claim_due(&mut tx, now + Duration::minutes(1), lease, 10, 5)
            .await?
            .is_empty(),
        "a lease holds the rows"
    );
    let again = alerts::claim_due(&mut tx, lease, lease + Duration::minutes(5), 10, 5).await?;
    assert_eq!(again.len(), 3, "the rows are due again after the lease");
    assert!(again.iter().all(|claim| claim.attempt == 0));
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

/// AC-MN-07: the outbox of a tenant is bounded.
#[tokio::test]
async fn ac_mn_07_db_the_outbox_is_bounded() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    let channel = email_channel(&mut tx, "mail").await?;
    let alert = open_alert(&mut tx, "agent:1", now).await?;
    sqlx::query(
        "INSERT INTO alert_deliveries \
         (id, tenant_id, alert_id, channel_id, event, next_attempt_at, created_at) \
         SELECT gen_random_uuid(), $1, $2, $3, 'opened', now(), now() \
         FROM generate_series(1, $4)",
    )
    .bind(tenant)
    .bind(alert)
    .bind(channel)
    .bind(alerts::MAX_PENDING_DELIVERIES as i32)
    .execute(tx.as_conn())
    .await?;
    alerts::enqueue(&mut tx, alert, DeliveryEvent::Resolved, now).await?;
    let counts = alerts::delivery_counts(&mut tx).await?;
    assert!(counts.contains(&(DeliveryStatus::Pending, alerts::MAX_PENDING_DELIVERIES)));
    assert!(counts.contains(&(DeliveryStatus::Failed, 1)), "{counts:?}");
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

/// The channel row keeps its kind rules: a webhook needs an origin and two secrets, an email needs
/// recipients.
#[tokio::test]
async fn db_a_channel_row_keeps_its_kind_rules() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let mut tx = db.database().begin_tenant(tenant).await?;
    let write = sqlx::query(
        "INSERT INTO notification_channels (id, tenant_id, name, kind, url_origin) \
         VALUES ($1, $2, 'hook', 'webhook', 'https://hooks.example.com')",
    )
    .bind(ChannelId::new())
    .bind(tenant)
    .execute(tx.as_conn())
    .await;
    assert_code(write, "23514");
    drop(tx);

    let mut tx = db.database().begin_tenant(tenant).await?;
    let write = sqlx::query(
        "INSERT INTO notification_channels (id, tenant_id, name, kind) \
         VALUES ($1, $2, 'mail', 'email')",
    )
    .bind(ChannelId::new())
    .bind(tenant)
    .execute(tx.as_conn())
    .await;
    assert_code(write, "23514");
    drop(tx);
    db.cleanup().await;
    Ok(())
}

/// AC-MN-07: a write of a holder whose lease ended, and whose row another sender claimed since,
/// changes nothing.
#[tokio::test]
async fn ac_mn_07_db_a_late_write_after_a_second_claim_changes_nothing() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    email_channel(&mut tx, "mail").await?;
    let alert = open_alert(&mut tx, "agent:1", now).await?;
    alerts::enqueue(&mut tx, alert, DeliveryEvent::Opened, now).await?;
    tx.commit().await?;

    let lease = now + Duration::minutes(2);
    let mut tx = db.database().begin_tenant(tenant).await?;
    let slow = alerts::claim_due(&mut tx, now, lease, 10, 5).await?;
    tx.commit().await?;
    assert_eq!(slow.len(), 1);

    // The lease ends before the first holder starts to send. A second holder claims the row and
    // starts its send first.
    let mut tx = db.database().begin_tenant(tenant).await?;
    let second = alerts::claim_due(&mut tx, lease, lease + Duration::minutes(2), 10, 5).await?;
    tx.commit().await?;
    assert_eq!(second.len(), 1);
    let mut tx = db.database().begin_tenant(tenant).await?;
    let id = second[0].id;
    assert_eq!(
        alerts::extend_lease(&mut tx, id, 0, lease + Duration::minutes(2)).await?,
        Some(1)
    );
    assert_eq!(
        alerts::extend_lease(&mut tx, id, 0, lease + Duration::hours(1)).await?,
        None,
        "the old holder cannot start a second send of the same attempt"
    );
    assert!(!alerts::mark_sent(&mut tx, id, 0, lease, None).await?);
    assert!(!alerts::mark_attempt_failed(&mut tx, id, 0, "late", None, lease).await?);
    assert!(!alerts::release(&mut tx, id, 0, lease).await?);
    let counts = alerts::delivery_counts(&mut tx).await?;
    assert_eq!(
        counts,
        vec![(DeliveryStatus::Pending, 1)],
        "the row is unchanged"
    );
    assert!(alerts::mark_sent(&mut tx, id, 1, lease, None).await?);
    assert!(
        !alerts::mark_sent(&mut tx, id, 1, lease, None).await?,
        "a sent row is final"
    );
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

/// AC-MN-07: a release keeps the attempts, and a due row at the limit ends as failed instead of
/// being claimed.
#[tokio::test]
async fn ac_mn_07_db_release_keeps_attempts_and_the_limit_ends_a_row() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    email_channel(&mut tx, "mail").await?;
    for index in 0..2 {
        let alert = open_alert(&mut tx, &format!("agent:{index}"), now).await?;
        alerts::enqueue(&mut tx, alert, DeliveryEvent::Opened, now).await?;
    }
    tx.commit().await?;

    let lease = now + Duration::minutes(2);
    let mut tx = db.database().begin_tenant(tenant).await?;
    let claimed = alerts::claim_due(&mut tx, now, lease, 10, 2).await?;
    assert_eq!(claimed.len(), 2);
    assert!(alerts::release(&mut tx, claimed[0].id, 0, now).await?);
    assert_eq!(
        alerts::extend_lease(&mut tx, claimed[1].id, 0, lease).await?,
        Some(1)
    );
    assert_eq!(
        alerts::extend_lease(&mut tx, claimed[1].id, 1, lease).await?,
        Some(2)
    );
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant).await?;
    let again = alerts::claim_due(&mut tx, lease, lease + Duration::minutes(2), 10, 2).await?;
    assert_eq!(again.len(), 1, "the row at the limit is not claimed");
    assert_eq!(again[0].id, claimed[0].id);
    assert_eq!(again[0].attempt, 0, "the released row kept its attempts");
    let counts = alerts::delivery_counts(&mut tx).await?;
    assert_eq!(
        counts,
        vec![(DeliveryStatus::Failed, 1), (DeliveryStatus::Pending, 1)]
    );
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

/// FR-MN-01: a stored job kind that this version does not know reads as `other`, so one row never
/// stops the evaluation of a tenant.
#[tokio::test]
async fn fr_mn_01_db_an_unknown_job_kind_in_params_reads_as_other() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    open_alert(&mut tx, "agent:1", now).await?;
    sqlx::query("UPDATE alert_state SET params = '{\"job_kind\":\"xyz\"}'::jsonb")
        .execute(tx.as_conn())
        .await?;
    let all = alerts::open_alerts(&mut tx).await?;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].params.0.job_kind, Some(JobKind::Other));
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

/// AC-MN-07: the evaluation reads every open alert, not the first thousands.
#[tokio::test]
async fn ac_mn_07_db_open_alerts_have_no_hard_limit() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    sqlx::query(
        "INSERT INTO alert_state (id, tenant_id, kind, dedupe_key, status, opened_at) \
         SELECT gen_random_uuid(), $1, 'agent_offline', 'agent:' || n, 'open', \
                $2 + n * interval '1 millisecond' \
         FROM generate_series(1, 5001) AS n",
    )
    .bind(tenant)
    .bind(now)
    .execute(tx.as_conn())
    .await?;
    let all = alerts::open_alerts(&mut tx).await?;
    assert_eq!(all.len(), 5001);
    let mut keys: Vec<&str> = all.iter().map(|alert| alert.dedupe_key.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 5001, "no row repeats between the pages");
    assert_eq!(alerts::count_open(&mut tx).await?, 5001);
    let newest = alerts::newest_open(&mut tx, 3).await?;
    assert_eq!(newest.len(), 3);
    assert_eq!(newest[0].dedupe_key, "agent:5001");
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}
