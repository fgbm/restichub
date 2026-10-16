//! The state tab counts, the name search and the new orders of the server lists: jobs, alerts and
//! the audit log with the resolved names (FR-UI-01, FR-SC-09, FR-AU-01). Each count and each name
//! stays inside its tenant (ADR-0006).
#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, AuditEvent};
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, JobId, PLATFORM_TENANT_ID, RepositoryId, TenantId};
use serde_json::Value;

use common::{
    app, create_tenant, create_tenant_with_key, create_user, request, seed_agent, seed_job,
    seed_plan, seed_profile, seed_repository, seed_secret, seed_session, send,
};

async fn exec(db: &TestDatabase, tenant: TenantId, sql: &str, id: uuid::Uuid) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect("cannot run the statement");
    tx.commit().await.expect("cannot commit the statement");
}

async fn get(app: &axum::Router, cookie: &str, uri: &str) -> Value {
    let (status, _, body) = send(app, request("GET", uri, Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("the page holds items")
        .iter()
        .map(|item| item["id"].as_str().expect("an item has an id").to_owned())
        .collect()
}

fn tabs(page: &Value, names: &[&str]) -> Vec<i64> {
    names
        .iter()
        .map(|name| {
            assert_eq!(page["counts"][name]["capped"], false, "{page}");
            page["counts"][name]["count"].as_i64().expect("a tab count")
        })
        .collect()
}

const JOB_TABS: [&str; 4] = ["running", "failed", "succeeded", "succeeded_with_warnings"];

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
async fn fr_sc_09_the_job_list_counts_the_state_tabs_searches_the_names_and_sorts_by_duration() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "job-tabs").await;
    let other = create_tenant_with_key(db.database(), "job-tabs-other").await;
    let secret = seed_secret(db.database(), tenant, "pw").await;
    let repo_a = seed_repository(db.database(), tenant, "repo-a", secret).await;
    let repo_b = seed_repository(db.database(), tenant, "repo_b", secret).await;
    let web = seed_agent(db.database(), tenant, "web-01").await;
    let database = seed_agent(db.database(), tenant, "db-01").await;
    let profile = seed_profile(db.database(), tenant, "files", repo_a).await;
    let nightly = seed_plan(db.database(), tenant, "Nightly", profile, web).await;
    let plan = format!("plan_id = '{nightly}', ");

    let running = job(
        &db,
        tenant,
        web,
        repo_a,
        &format!("{plan}kind = 'backup', status = 'running', started_at = now()"),
    )
    .await;
    let failed = job(
        &db,
        tenant,
        database,
        repo_b,
        "kind = 'check', status = 'failed', result_code = 'failed', \
         started_at = now() - interval '3 hours', finished_at = now() - interval '175 minutes'",
    )
    .await;
    job(
        &db,
        tenant,
        database,
        repo_b,
        "status = 'failed', result_code = 'cancelled', finished_at = now()",
    )
    .await;
    let dead = job(
        &db,
        tenant,
        web,
        repo_a,
        &format!("{plan}kind = 'backup', status = 'dead_letter', finished_at = now()"),
    )
    .await;
    let succeeded = job(
        &db,
        tenant,
        web,
        repo_a,
        &format!(
            "{plan}kind = 'backup', status = 'succeeded', result_code = 'succeeded', \
             started_at = now() - interval '62 minutes', finished_at = now() - interval '1 hour'"
        ),
    )
    .await;
    let warnings = job(
        &db,
        tenant,
        database,
        repo_b,
        "kind = 'backup', status = 'succeeded', result_code = 'succeeded_with_warnings', \
         started_at = now() - interval '150 minutes', finished_at = now() - interval '2 hours'",
    )
    .await;
    job(&db, tenant, web, repo_a, "kind = 'backup'").await;
    let other_secret = seed_secret(db.database(), other, "pw").await;
    let other_repo = seed_repository(db.database(), other, "repo-a", other_secret).await;
    let other_host = seed_agent(db.database(), other, "web-01").await;
    for _ in 0..3 {
        job(
            &db,
            other,
            other_host,
            other_repo,
            "kind = 'backup', status = 'failed', finished_at = now()",
        )
        .await;
    }
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let page = get(&app, &viewer, "/api/v1/jobs").await;
    assert_eq!(page["total"], 7);
    assert_eq!(tabs(&page, &JOB_TABS), [1, 2, 1, 1]);

    // A tab filters the list and keeps the counts of the other tabs.
    let page = get(&app, &viewer, "/api/v1/jobs?group=failed").await;
    assert_eq!(ids(&page), [dead, failed].map(|id| id.to_string()));
    assert_eq!(tabs(&page, &JOB_TABS), [1, 2, 1, 1]);
    let page = get(&app, &viewer, "/api/v1/jobs?group=succeeded_with_warnings").await;
    assert_eq!(ids(&page), [warnings.to_string()]);
    let page = get(&app, &viewer, "/api/v1/jobs?group=succeeded").await;
    assert_eq!(ids(&page), [succeeded.to_string()]);
    let page = get(&app, &viewer, "/api/v1/jobs?status=running").await;
    assert_eq!(tabs(&page, &JOB_TABS), [1, 2, 1, 1]);

    // The counts follow the other filters.
    let page = get(&app, &viewer, &format!("/api/v1/jobs?agent_id={database}")).await;
    assert_eq!(tabs(&page, &JOB_TABS), [0, 1, 0, 1]);

    // The search matches the plan, the host or the repository name; a wildcard is literal.
    let page = get(&app, &viewer, "/api/v1/jobs?q=NIGHT").await;
    assert_eq!(page["total"], 3);
    assert_eq!(tabs(&page, &JOB_TABS), [1, 1, 1, 0]);
    let page = get(&app, &viewer, "/api/v1/jobs?q=db-0").await;
    assert_eq!(page["total"], 3);
    let page = get(&app, &viewer, "/api/v1/jobs?q=repo_").await;
    assert_eq!(page["total"], 3);
    let page = get(&app, &viewer, "/api/v1/jobs?q=repo%25").await;
    assert_eq!(page["total"], 0);

    // The longest finished job comes first; an unfinished job is last in both orders.
    let page = get(
        &app,
        &viewer,
        "/api/v1/jobs?sort=duration&order=desc&limit=3",
    )
    .await;
    assert_eq!(
        ids(&page),
        [warnings, failed, succeeded].map(|id| id.to_string())
    );
    let page = get(&app, &viewer, "/api/v1/jobs?sort=duration&order=asc").await;
    assert_eq!(
        ids(&page)[..3],
        [succeeded, failed, warnings].map(|id| id.to_string())
    );
    assert!(!ids(&page)[..3].contains(&running.to_string()));
    let page = get(
        &app,
        &viewer,
        "/api/v1/jobs?sort=finished_at&order=desc&limit=3",
    )
    .await;
    assert_eq!(page["items"][2]["id"], succeeded.to_string());
    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_job_tab_and_search_parameters_give_stable_codes() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "job-tab-bounds").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());
    let long = "a".repeat(201);
    for (uri, field, reason) in [
        (
            "/api/v1/jobs?group=sleeping".to_owned(),
            "group",
            "invalid_format",
        ),
        (format!("/api/v1/jobs?q={long}"), "q", "too_long"),
        (
            "/api/v1/jobs?sort=length".to_owned(),
            "sort",
            "invalid_format",
        ),
        (format!("/api/v1/audit?q={long}"), "q", "too_long"),
    ] {
        let (status, _, body) = send(&app, request("GET", &uri, Some(&viewer), None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["field"], field, "{uri}: {body}");
        assert_eq!(body["reason"], reason, "{uri}: {body}");
    }
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/jobs?group=failed", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    db.cleanup().await;
}

async fn seed_alerts(
    db: &TestDatabase,
    tenant: TenantId,
    status: &str,
    hostname: &str,
    count: i64,
) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(
        "INSERT INTO alert_state (id, tenant_id, kind, dedupe_key, status, agent_hostname, \
                                  opened_at, resolved_at) \
         SELECT gen_random_uuid(), $1, 'agent_offline', $2 || $3 || n, $2, $3, now(), \
                CASE WHEN $2 = 'resolved' THEN now() END \
           FROM generate_series(1, $4) AS n",
    )
    .bind(tenant.as_uuid())
    .bind(status)
    .bind(hostname)
    .bind(count)
    .execute(&mut *tx)
    .await
    .expect("cannot write the alerts");
    tx.commit().await.expect("cannot commit the alerts");
}

#[tokio::test]
async fn fr_ui_01_the_alert_list_counts_the_open_and_the_closed_tab() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "alert-tabs").await;
    let other = create_tenant(db.database(), "alert-tabs-other").await;
    seed_alerts(&db, tenant, "open", "web-01", 2).await;
    seed_alerts(&db, tenant, "resolved", "web-01", 3).await;
    seed_alerts(&db, tenant, "resolved", "db-01", 4).await;
    seed_alerts(&db, other, "open", "web-01", 5).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let page = get(&app, &viewer, "/api/v1/alerts").await;
    assert_eq!(page["total"], 2);
    assert_eq!(tabs(&page, &["open", "resolved"]), [2, 7]);
    let page = get(&app, &viewer, "/api/v1/alerts?status=resolved&q=WEB").await;
    assert_eq!(page["total"], 3);
    assert_eq!(tabs(&page, &["open", "resolved"]), [2, 3]);
    let page = get(&app, &viewer, "/api/v1/alerts?kind=job_failed").await;
    assert_eq!(tabs(&page, &["open", "resolved"]), [0, 0]);
    db.cleanup().await;
}

fn record<'a>(page: &'a Value, action: &str) -> &'a Value {
    page["items"]
        .as_array()
        .expect("the page holds items")
        .iter()
        .find(|item| item["action"] == action)
        .unwrap_or_else(|| panic!("no record {action}: {page}"))
}

#[tokio::test]
async fn fr_au_01_the_audit_list_names_the_object_and_the_actor_and_searches_them() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "audit-names").await;
    let other = create_tenant_with_key(db.database(), "audit-names-other").await;
    let secret = seed_secret(db.database(), tenant, "pw").await;
    let repository = seed_repository(db.database(), tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), tenant, "user-profiles", repository).await;
    let host = seed_agent(db.database(), tenant, "web-01").await;
    let plan = seed_plan(db.database(), tenant, "nightly", profile, host).await;
    let run = seed_job(db.database(), tenant, host, repository).await;
    exec(
        &db,
        tenant,
        &format!("UPDATE jobs SET plan_id = '{plan}' WHERE id = $1"),
        run.as_uuid(),
    )
    .await;
    let anna = create_user(db.database(), tenant, "anna", "phc", "operator").await;
    let records = [
        (
            tenant,
            Actor::User(anna),
            AuditAction::ProfileUpdate,
            "profile",
            profile.as_uuid(),
        ),
        (
            tenant,
            Actor::Agent(host.as_uuid()),
            AuditAction::JobRun,
            "job",
            run.as_uuid(),
        ),
        (
            tenant,
            Actor::System,
            AuditAction::RepositoryDelete,
            "repository",
            uuid::Uuid::now_v7(),
        ),
        // A record of another tenant that names an object of this tenant gets no name.
        (
            other,
            Actor::User(anna),
            AuditAction::ProfileUpdate,
            "profile",
            profile.as_uuid(),
        ),
    ];
    for (scope, actor, action, object_type, object_id) in records {
        let mut tx = db
            .database()
            .begin_tenant(scope)
            .await
            .expect("cannot begin");
        let mut event = AuditEvent::new(action, object_type);
        event.actor = actor;
        event.object_id = Some(object_id);
        restichub_db::audit::insert(&mut tx, &event)
            .await
            .expect("cannot write the record");
        tx.commit().await.expect("cannot commit the record");
    }
    let mut tx = db.database().begin_platform().await.expect("cannot begin");
    let mut event = AuditEvent::new(AuditAction::TenantSuspend, "tenant");
    event.object_id = Some(tenant.as_uuid());
    restichub_db::audit::insert_platform_for(&mut tx, tenant, &event)
        .await
        .expect("cannot write the platform record");
    tx.commit()
        .await
        .expect("cannot commit the platform record");
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let other_viewer = seed_session(db.database(), other, "other-viewer", "viewer").await;
    let root = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app(db.database());

    let page = get(&app, &viewer, "/api/v1/audit").await;
    let update = record(&page, "profile_update");
    assert_eq!(update["object_name"], "user-profiles");
    assert_eq!(update["actor_name"], "anna");
    let job_run = record(&page, "job_run");
    assert_eq!(job_run["object_name"], "nightly");
    assert_eq!(job_run["actor_name"], "web-01");
    let removed = record(&page, "repository_delete");
    assert!(removed["object_name"].is_null());
    assert!(removed["actor_name"].is_null());
    assert_eq!(
        record(&page, "tenant_suspend")["object_name"],
        "audit-names"
    );

    for (query, action) in [
        ("USER-PROF", "profile_update"),
        ("ann", "profile_update"),
        ("web-0", "job_run"),
        ("nightly", "job_run"),
        ("repository_del", "repository_delete"),
    ] {
        let page = get(&app, &viewer, &format!("/api/v1/audit?q={query}")).await;
        assert_eq!(page["total"], 1, "{query}: {page}");
        assert_eq!(page["items"][0]["action"], action, "{query}");
    }
    let page = get(&app, &viewer, "/api/v1/audit?q=%25").await;
    assert_eq!(page["total"], 0);

    let page = get(&app, &other_viewer, "/api/v1/audit").await;
    assert!(record(&page, "profile_update")["object_name"].is_null());
    let page = get(&app, &other_viewer, "/api/v1/audit?q=user-prof").await;
    assert_eq!(page["total"], 0);

    // The platform list sees the accounts and the tenants, not the objects inside a tenant.
    let page = get(
        &app,
        &root,
        &format!("/api/v1/platform/audit?tenant_id={tenant}"),
    )
    .await;
    let update = record(&page, "profile_update");
    assert!(update["object_name"].is_null());
    assert_eq!(update["actor_name"], "anna");
    assert_eq!(
        record(&page, "tenant_suspend")["object_name"],
        "audit-names"
    );
    let page = get(&app, &root, "/api/v1/platform/audit?q=audit-names").await;
    let items = page["items"].as_array().expect("the page holds items");
    assert!(!items.is_empty());
    assert!(items.iter().all(|item| {
        item["object_name"]
            .as_str()
            .is_some_and(|name| name.starts_with("audit-names"))
    }));
    db.cleanup().await;
}
