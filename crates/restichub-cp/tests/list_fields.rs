//! The facts that the entity lists show in each item: the plan RPO state, the repository health,
//! the last delivery of a channel, the last sign-in of a user and the host count of an
//! organization (FR-UI-01, ADR-0036). Each fact stays inside its tenant (ADR-0006).
#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, AuditEvent};
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, JobId, PLATFORM_TENANT_ID, RepositoryId, TenantId};
use serde_json::Value;

use common::{
    app, create_tenant, create_tenant_with_key, create_user, exec, request, seed_agent, seed_job,
    seed_plan, seed_profile, seed_repository, seed_secret, seed_session, send, session_cookie,
};

async fn get(app: &axum::Router, cookie: &str, uri: &str) -> Value {
    let (status, _, body) = send(app, request("GET", uri, Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

fn item<'a>(page: &'a Value, field: &str, value: &str) -> &'a Value {
    page["items"]
        .as_array()
        .expect("the page holds items")
        .iter()
        .find(|item| item[field] == value)
        .unwrap_or_else(|| panic!("no item with {field} = {value}: {page}"))
}

/// A job of the agent on the repository with the columns that `set` gives.
async fn job(
    db: &TestDatabase,
    tenant: TenantId,
    agent: AgentId,
    repository: RepositoryId,
    set: &str,
) -> JobId {
    let id = seed_job(db.database(), tenant, agent, repository).await;
    exec(
        db,
        tenant,
        &format!("UPDATE jobs SET {set} WHERE id = $1"),
        id.as_uuid(),
    )
    .await;
    id
}

#[tokio::test]
async fn fr_ui_01_the_plan_list_shows_the_rpo_state_and_the_last_success() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-facts").await;
    let other = create_tenant_with_key(db.database(), "plan-facts-other").await;
    let secret = seed_secret(db.database(), tenant, "pw").await;
    let repository = seed_repository(db.database(), tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), tenant, "files", repository).await;
    let host = seed_agent(db.database(), tenant, "web-01").await;
    let gone = seed_agent(db.database(), tenant, "old-01").await;
    let fresh = seed_plan(db.database(), tenant, "fresh", profile, host).await;
    let late = seed_plan(db.database(), tenant, "late", profile, host).await;
    let never = seed_plan(db.database(), tenant, "never", profile, host).await;
    let orphan = seed_plan(db.database(), tenant, "orphan", profile, gone).await;
    // The daily schedule gives an objective of 26 hours; 75 % of it is 19.5 hours.
    for (plan, age) in [(fresh, "1 hour"), (late, "20 hours")] {
        let id = seed_job(db.database(), tenant, host, repository).await;
        exec(
            &db,
            tenant,
            &format!(
                "UPDATE jobs SET kind = 'backup', status = 'succeeded', result_code = 'succeeded', \
                 plan_id = '{plan}', started_at = now() - interval '{age}', \
                 finished_at = now() - interval '{age}' WHERE id = $1"
            ),
            id.as_uuid(),
        )
        .await;
    }
    exec(
        &db,
        tenant,
        "UPDATE plans SET created_at = now() - interval '2 days' WHERE id = $1",
        never.as_uuid(),
    )
    .await;
    exec(
        &db,
        tenant,
        "UPDATE agents SET status = 'revoked', revoked_at = now() WHERE id = $1",
        gone.as_uuid(),
    )
    .await;
    // A success in another tenant never counts for a plan of this tenant.
    let other_secret = seed_secret(db.database(), other, "pw").await;
    let other_repository = seed_repository(db.database(), other, "repo", other_secret).await;
    let other_host = seed_agent(db.database(), other, "web-01").await;
    let foreign = seed_job(db.database(), other, other_host, other_repository).await;
    exec(
        &db,
        other,
        "UPDATE jobs SET kind = 'backup', status = 'succeeded', finished_at = now() WHERE id = $1",
        foreign.as_uuid(),
    )
    .await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let page = get(&app, &viewer, "/api/v1/plans").await;
    assert_eq!(item(&page, "name", "fresh")["rpo_state"], "ok");
    assert!(item(&page, "name", "fresh")["last_success_at"].is_string());
    assert_eq!(item(&page, "name", "late")["rpo_state"], "at_risk");
    assert_eq!(item(&page, "name", "never")["rpo_state"], "violated");
    assert!(item(&page, "name", "never")["last_success_at"].is_null());
    assert_eq!(item(&page, "name", "orphan")["rpo_state"], "unknown");

    let plan = get(&app, &viewer, &format!("/api/v1/plans/{orphan}")).await;
    assert_eq!(plan["rpo_state"], "unknown");
    let plan = get(&app, &viewer, &format!("/api/v1/plans/{late}")).await;
    assert_eq!(plan["rpo_state"], "at_risk");
    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_repository_list_shows_the_lock_the_last_check_and_the_snapshot_count() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "repo-facts").await;
    let other = create_tenant_with_key(db.database(), "repo-facts-other").await;
    let secret = seed_secret(db.database(), tenant, "pw").await;
    let locked = seed_repository(db.database(), tenant, "locked", secret).await;
    let checked = seed_repository(db.database(), tenant, "checked", secret).await;
    let empty = seed_repository(db.database(), tenant, "empty", secret).await;
    let host = seed_agent(db.database(), tenant, "web-01").await;

    job(
        &db,
        tenant,
        host,
        locked,
        "kind = 'check', status = 'failed', result_code = 'failed', \
         started_at = now() - interval '2 hours', finished_at = now() - interval '1 hour'",
    )
    .await;
    // A cancelled check is newer, but it says nothing about the data.
    job(
        &db,
        tenant,
        host,
        locked,
        "kind = 'check', status = 'failed', result_code = 'cancelled', finished_at = now()",
    )
    .await;
    job(
        &db,
        tenant,
        host,
        locked,
        "kind = 'backup', stale_lock_since = now() - interval '1 hour'",
    )
    .await;
    job(
        &db,
        tenant,
        host,
        checked,
        "kind = 'check', status = 'succeeded', result_code = 'succeeded', finished_at = now()",
    )
    .await;
    job(
        &db,
        tenant,
        host,
        checked,
        "kind = 'backup', status = 'running', attempt = 1",
    )
    .await;
    // A listing with a filter counts a part of the repository, also when it is the newest one.
    let all = r#"{"kind": "snapshots", "hosts": [], "tags": [], "paths": []}"#;
    let one_host = r#"{"kind": "snapshots", "hosts": ["web-01"], "tags": [], "paths": []}"#;
    let one_tag = r#"{"kind": "snapshots", "hosts": [], "tags": ["db"], "paths": []}"#;
    for (repository, count, age, request) in [
        (checked, 60, "10 minutes", one_host),
        (checked, 146, "1 hour", all),
        (checked, 9, "1 day", all),
        (empty, 12, "10 minutes", one_tag),
    ] {
        let truncated = false;
        let listing = job(
            &db,
            tenant,
            host,
            repository,
            &format!(
                "kind = 'snapshots', status = 'succeeded', finished_at = now() - interval '{age}'"
            ),
        )
        .await;
        exec(
            &db,
            tenant,
            &format!(
                "INSERT INTO job_steps (tenant_id, job_id, step_id, status, exit_code, output_kind, \
                                        summary) \
                 SELECT tenant_id, id, 'snapshots', 'success', 0, 'snapshots', \
                        '{{\"count\": {count}, \"truncated\": {truncated}}}'::jsonb \
                 FROM jobs WHERE id = $1"
            ),
            listing.as_uuid(),
        )
        .await;
        exec(
            &db,
            tenant,
            &format!(
                "INSERT INTO job_requests (tenant_id, job_id, request) \
                 SELECT tenant_id, id, '{request}'::jsonb FROM jobs WHERE id = $1"
            ),
            listing.as_uuid(),
        )
        .await;
    }
    // The same names in another tenant hold other facts.
    let other_secret = seed_secret(db.database(), other, "pw").await;
    let other_repository = seed_repository(db.database(), other, "empty", other_secret).await;
    let other_host = seed_agent(db.database(), other, "web-01").await;
    job(
        &db,
        other,
        other_host,
        other_repository,
        "kind = 'check', status = 'failed', finished_at = now()",
    )
    .await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let page = get(&app, &viewer, "/api/v1/repositories").await;
    let item_locked = item(&page, "name", "locked");
    assert_eq!(item_locked["lock_state"], "stale");
    assert_eq!(item_locked["last_check"]["result"], "failed");
    assert!(item_locked["snapshot_count"].is_null());
    let item_checked = item(&page, "name", "checked");
    assert_eq!(item_checked["lock_state"], "held");
    assert_eq!(item_checked["last_check"]["result"], "passed");
    assert_eq!(item_checked["snapshot_count"], 146);
    let item_empty = item(&page, "name", "empty");
    assert_eq!(item_empty["lock_state"], "free");
    assert!(item_empty["last_check"].is_null());
    assert!(item_empty["snapshot_count"].is_null());

    let one = get(&app, &viewer, &format!("/api/v1/repositories/{empty}")).await;
    assert!(one["last_check"].is_null());
    let one = get(&app, &viewer, &format!("/api/v1/repositories/{locked}")).await;
    assert_eq!(one["lock_state"], "stale");
    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_channel_list_shows_the_last_delivery_attempt() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "channel-facts").await;
    let other = create_tenant(db.database(), "channel-facts-other").await;
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    for (scope, channels) in [
        (tenant, vec!["ops", "quiet", "flaky"]),
        (other, vec!["ops"]),
    ] {
        sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
            .bind(scope.to_string())
            .execute(&mut *tx)
            .await
            .expect("cannot set the tenant scope");
        sqlx::query(
            "INSERT INTO alert_state (id, tenant_id, kind, dedupe_key, status, opened_at) \
             VALUES (gen_random_uuid(), $1, 'agent_offline', 'a', 'open', now())",
        )
        .bind(scope.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("cannot write the alert");
        for name in channels {
            sqlx::query(
                "INSERT INTO notification_channels (id, tenant_id, name, kind, recipients) \
                 VALUES (gen_random_uuid(), $1, $2, 'email', ARRAY['ops@example.com'])",
            )
            .bind(scope.as_uuid())
            .bind(name)
            .execute(&mut *tx)
            .await
            .expect("cannot write the channel");
        }
        // The owner connection reads the rows of the scope only, so each scope writes its own.
        // `ops` failed first and then sent; `flaky` waits for a retry after a failed attempt; the
        // other tenant has a newer failure of a channel with the same name.
        sqlx::query(
            "INSERT INTO alert_deliveries (id, tenant_id, alert_id, channel_id, event, status, \
                                           attempts, next_attempt_at, created_at, sent_at, attempted_at) \
             SELECT gen_random_uuid(), c.tenant_id, s.id, c.id, 'opened', d.status, 1, now(), now(), \
                    d.sent_at, d.attempted_at \
             FROM notification_channels c \
             JOIN alert_state s ON s.tenant_id = c.tenant_id \
             JOIN (VALUES ($1::uuid, 'ops', 'failed', NULL::timestamptz, now() - interval '2 hours'), \
                          ($1, 'ops', 'sent', now() - interval '1 hour', now() - interval '1 hour'), \
                          ($1, 'flaky', 'pending', NULL, now() - interval '5 minutes'), \
                          ($2::uuid, 'ops', 'failed', NULL, now())) \
                  AS d(tenant_id, name, status, sent_at, attempted_at) \
               ON d.tenant_id = c.tenant_id AND d.name = c.name",
        )
        .bind(tenant.as_uuid())
        .bind(other.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("cannot write the deliveries");
    }
    tx.commit().await.expect("cannot commit the deliveries");
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app(db.database());

    let page = get(&app, &admin, "/api/v1/notification-channels").await;
    assert_eq!(item(&page, "name", "ops")["last_delivery_ok"], true);
    assert!(item(&page, "name", "ops")["last_delivery_at"].is_string());
    assert_eq!(item(&page, "name", "flaky")["last_delivery_ok"], false);
    assert!(item(&page, "name", "quiet")["last_delivery_at"].is_null());
    assert!(item(&page, "name", "quiet")["last_delivery_ok"].is_null());
    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_user_list_shows_the_last_sign_in_into_the_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "user-facts").await;
    let other = create_tenant(db.database(), "user-facts-other").await;
    let anna = create_user(db.database(), tenant, "anna", "phc", "operator").await;
    let boris = create_user(db.database(), tenant, "boris", "phc", "viewer").await;
    let admin = create_user(db.database(), tenant, "admin", "phc", "admin").await;
    let other_admin = create_user(db.database(), other, "beth", "phc", "admin").await;
    let mut tx = db.database().begin_platform().await.expect("cannot begin");
    let added =
        restichub_db::auth::add_membership(&mut tx, anna, other, restichub_db::Role::Viewer)
            .await
            .expect("cannot add the membership");
    assert!(added, "the membership must be added");
    tx.commit().await.expect("cannot commit the membership");
    for (scope, actor) in [
        (tenant, anna),
        // A record of another tenant never shows, also for the same account.
        (other, boris),
    ] {
        let mut tx = db
            .database()
            .begin_tenant(scope)
            .await
            .expect("cannot begin");
        let mut event = AuditEvent::new(AuditAction::SignIn, "user");
        event.actor = Actor::User(actor);
        event.object_id = Some(actor.as_uuid());
        restichub_db::audit::insert(&mut tx, &event)
            .await
            .expect("cannot write the record");
        tx.commit().await.expect("cannot commit the record");
    }
    let cookie = session_cookie(db.database(), tenant, admin).await;
    let other_cookie = session_cookie(db.database(), other, other_admin).await;
    let app = app(db.database());
    let last_sign_in = async |cookie: &str| {
        let page = get(&app, cookie, "/api/v1/users").await;
        item(&page, "username", "anna")["last_sign_in_at"].clone()
    };
    let switch = async |cookie: &str, target: TenantId| {
        let (status, cookie, body) = send(
            &app,
            request(
                "POST",
                "/api/v1/auth/switch",
                Some(cookie),
                Some(serde_json::json!({"tenant_id": target.to_string()})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        cookie.expect("a rotated session cookie")
    };

    let page = get(&app, &cookie, "/api/v1/users").await;
    assert!(item(&page, "username", "admin")["last_sign_in_at"].is_null());
    assert!(item(&page, "username", "boris")["last_sign_in_at"].is_null());
    let signed_in = last_sign_in(&cookie).await;
    assert!(signed_in.is_string(), "{page}");
    assert!(last_sign_in(&other_cookie).await.is_null());

    // A switch out of the tenant is not a sign-in into it; a switch into a tenant is.
    let anna_cookie = session_cookie(db.database(), tenant, anna).await;
    let anna_cookie = switch(&anna_cookie, other).await;
    assert_eq!(last_sign_in(&cookie).await, signed_in);
    let switched_in = last_sign_in(&other_cookie).await;
    assert!(switched_in.is_string());
    switch(&anna_cookie, tenant).await;
    let back = last_sign_in(&cookie).await;
    let time = |value: &Value| {
        let text = value.as_str().expect("a sign-in time");
        time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
            .expect("an RFC 3339 time")
    };
    assert!(time(&back) > time(&signed_in), "{back} after {signed_in}");
    assert_eq!(last_sign_in(&other_cookie).await, switched_in);
    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_platform_tenant_list_counts_the_hosts_that_are_not_revoked() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let busy = create_tenant(db.database(), "hosts-busy").await;
    let idle = create_tenant(db.database(), "hosts-idle").await;
    seed_agent(db.database(), busy, "web-01").await;
    seed_agent(db.database(), busy, "web-02").await;
    let revoked = seed_agent(db.database(), busy, "old-01").await;
    exec(
        &db,
        busy,
        "UPDATE agents SET status = 'revoked', revoked_at = now() WHERE id = $1",
        revoked.as_uuid(),
    )
    .await;
    let root = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let tenant_admin = seed_session(db.database(), busy, "admin", "admin").await;
    let app = app(db.database());

    let page = get(&app, &root, "/api/v1/platform/tenants").await;
    assert_eq!(item(&page, "name", "hosts-busy")["agent_count"], 2);
    assert_eq!(item(&page, "name", "hosts-idle")["agent_count"], 0);
    let one = get(&app, &root, &format!("/api/v1/platform/tenants/{idle}")).await;
    assert_eq!(one["agent_count"], 0);

    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/platform/tenants", Some(&tenant_admin), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    db.cleanup().await;
}
