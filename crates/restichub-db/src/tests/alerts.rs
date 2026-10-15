use super::*;
use crate::alerts::{self, AlertKind, DeliveryEvent, DeliveryStatus, NewAlert};
use crate::channels::{self, ChannelKind, NewChannel};
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
            url: None,
            secret_id: None,
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
            detail: "silent",
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
    tx.savepoint("second").await?;
    let second = open_alert(&mut tx, "agent:1", now).await;
    assert!(
        second
            .as_ref()
            .is_err_and(crate::DbError::is_unique_violation),
        "a second open alert with the same key is refused"
    );
    tx.rollback_to_savepoint("second").await?;
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
    let claimed_first = alerts::claim_due(&mut first, now, lease, 2).await?;
    let claimed_second = alerts::claim_due(&mut second, now, lease, 2).await?;
    assert_eq!(claimed_first.len(), 2);
    assert_eq!(claimed_second.len(), 1, "the locked rows are skipped");
    for one in &claimed_first {
        assert!(claimed_second.iter().all(|other| other.id != one.id));
        assert_eq!(one.attempt, 1);
    }
    first.commit().await?;
    second.commit().await?;

    let mut tx = db.database().begin_tenant(tenant).await?;
    assert!(
        alerts::claim_due(&mut tx, now + Duration::minutes(1), lease, 10)
            .await?
            .is_empty(),
        "a lease holds the rows"
    );
    let again = alerts::claim_due(&mut tx, lease, lease + Duration::minutes(5), 10).await?;
    assert_eq!(again.len(), 3, "the rows are due again after the lease");
    assert!(again.iter().all(|claim| claim.attempt == 2));
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

/// The channel row keeps its kind rules: a webhook needs a URL and a secret, an email needs
/// recipients.
#[tokio::test]
async fn db_a_channel_row_keeps_its_kind_rules() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    let mut tx = db.database().begin_tenant(tenant).await?;
    let write = sqlx::query(
        "INSERT INTO notification_channels (id, tenant_id, name, kind, url) \
         VALUES ($1, $2, 'hook', 'webhook', 'https://hooks.example.com/x')",
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
