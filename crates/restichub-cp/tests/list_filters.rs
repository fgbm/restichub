//! The host list and the job list filter, sort and page on the server (FR-UI-01, FR-SC-09).
#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, JobId, TenantId};
use serde_json::Value;
use time::OffsetDateTime;

use common::{
    app, create_tenant, create_tenant_with_key, request, seed_agent, seed_job, seed_repository,
    seed_secret, seed_session, send,
};

/// Runs one statement on a row of the tenant through the owner connection.
async fn update(db: &TestDatabase, tenant: TenantId, sql: &'static str, id: uuid::Uuid) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(sql)
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect("cannot update the row");
    tx.commit().await.expect("cannot commit the update");
}

async fn get(app: &axum::Router, cookie: &str, uri: &str) -> Value {
    let (status, _, body) = send(app, request("GET", uri, Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

fn names(page: &Value, field: &str) -> Vec<String> {
    page.get("items")
        .and_then(Value::as_array)
        .expect("the page holds items")
        .iter()
        .map(|item| item[field].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Four hosts, one in each state: `web-01` online on Linux with the label `web`, `db-01` offline on
/// Linux, `win-01` waiting on Windows, `old-01` revoked.
async fn seed_hosts(db: &TestDatabase, tenant: TenantId) -> [AgentId; 4] {
    let web = seed_agent(db.database(), tenant, "web-01").await;
    let database = seed_agent(db.database(), tenant, "db-01").await;
    let windows = seed_agent(db.database(), tenant, "win-01").await;
    let old = seed_agent(db.database(), tenant, "old-01").await;
    update(
        db,
        tenant,
        "UPDATE agents SET first_connected_at = now() - interval '1 day', last_seen_at = now(), \
         labels = ARRAY['web','prod'], agent_version = '0.4.2', server_scripts = 'enabled' \
         WHERE id = $1",
        web.as_uuid(),
    )
    .await;
    update(
        db,
        tenant,
        "UPDATE agents SET first_connected_at = now() - interval '2 days', \
         last_seen_at = now() - interval '1 hour', agent_version = '0.4.1' WHERE id = $1",
        database.as_uuid(),
    )
    .await;
    update(
        db,
        tenant,
        "UPDATE agents SET os = 'windows' WHERE id = $1",
        windows.as_uuid(),
    )
    .await;
    update(
        db,
        tenant,
        "UPDATE agents SET status = 'revoked', revoked_at = now(), \
         first_connected_at = now() - interval '9 days' WHERE id = $1",
        old.as_uuid(),
    )
    .await;
    [web, database, windows, old]
}

#[tokio::test]
async fn fr_ui_01_the_host_list_filters_by_state_system_version_label_scripts_and_search() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "host-filters").await;
    seed_hosts(&db, tenant).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let page = get(
        &app,
        &viewer,
        "/api/v1/agents?state=online,offline,waiting&sort=hostname&order=asc",
    )
    .await;
    assert_eq!(names(&page, "hostname"), ["db-01", "web-01", "win-01"]);
    assert_eq!(page["total"], 3);

    let page = get(&app, &viewer, "/api/v1/agents?state=revoked").await;
    assert_eq!(names(&page, "hostname"), ["old-01"]);
    let page = get(&app, &viewer, "/api/v1/agents?os=windows").await;
    assert_eq!(names(&page, "hostname"), ["win-01"]);
    let page = get(
        &app,
        &viewer,
        "/api/v1/agents?agent_version=0.4.1,0.4.2&sort=hostname&order=asc",
    )
    .await;
    assert_eq!(names(&page, "hostname"), ["db-01", "web-01"]);
    let page = get(&app, &viewer, "/api/v1/agents?label=prod").await;
    assert_eq!(names(&page, "hostname"), ["web-01"]);
    let page = get(&app, &viewer, "/api/v1/agents?server_scripts=enabled").await;
    assert_eq!(names(&page, "hostname"), ["web-01"]);
    let page = get(&app, &viewer, "/api/v1/agents?server_scripts=disabled&state=online,offline,waiting&sort=hostname&order=asc").await;
    assert_eq!(names(&page, "hostname"), ["db-01", "win-01"]);

    // The search matches a part of the name or of a label in any case; a wildcard is literal.
    let page = get(&app, &viewer, "/api/v1/agents?q=WEB").await;
    assert_eq!(names(&page, "hostname"), ["web-01"]);
    let page = get(&app, &viewer, "/api/v1/agents?q=%25").await;
    assert_eq!(page["total"], 0);

    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_host_list_counts_the_states_and_offers_the_filter_values() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "host-facets").await;
    seed_hosts(&db, tenant).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    // The state counts ignore the state filter, so each chip shows what it adds.
    let page = get(&app, &viewer, "/api/v1/agents?state=online&facets=true").await;
    assert_eq!(page["total"], 1);
    let states = &page["facets"]["states"];
    assert_eq!(
        (states["online"].as_i64(), states["offline"].as_i64()),
        (Some(1), Some(1))
    );
    assert_eq!(
        (states["waiting"].as_i64(), states["revoked"].as_i64()),
        (Some(1), Some(1))
    );
    let os = page["facets"]["os"].as_array().expect("the systems");
    assert!(
        os.iter()
            .any(|item| item["value"] == "linux" && item["count"] == 3)
    );
    let labels = page["facets"]["labels"].as_array().expect("the labels");
    assert!(
        labels
            .iter()
            .any(|item| item["value"] == "web" && item["count"] == 1)
    );

    // A system filter narrows the state counts too.
    let page = get(&app, &viewer, "/api/v1/agents?os=windows&facets=true").await;
    assert_eq!(page["facets"]["states"]["waiting"], 1);
    assert_eq!(page["facets"]["states"]["online"], 0);

    // Without the parameter the response holds no facets.
    let page = get(&app, &viewer, "/api/v1/agents").await;
    assert!(page["facets"].is_null());

    // A Viewer reads the lists; a request without a session reads nothing.
    for uri in ["/api/v1/agents?facets=true", "/api/v1/jobs?offset=0"] {
        let (status, _, _) = send(&app, request("GET", uri, None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_host_list_reads_numbered_pages_and_keeps_the_cursor_of_the_default_order() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "host-pages").await;
    seed_hosts(&db, tenant).await;
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app(db.database());

    let first = get(
        &app,
        &admin,
        "/api/v1/agents?sort=hostname&order=asc&limit=2",
    )
    .await;
    assert_eq!(names(&first, "hostname"), ["db-01", "old-01"]);
    assert_eq!(first["total"], 4);
    assert!(
        first["next_cursor"].is_null(),
        "another order gives no cursor"
    );
    let second = get(
        &app,
        &admin,
        "/api/v1/agents?sort=hostname&order=asc&limit=2&offset=2",
    )
    .await;
    assert_eq!(names(&second, "hostname"), ["web-01", "win-01"]);

    // The default order still pages with a cursor, so a client that reads the whole list works.
    let first = get(&app, &admin, "/api/v1/agents?limit=3").await;
    let cursor = first["next_cursor"].as_str().expect("a cursor").to_owned();
    let rest = get(
        &app,
        &admin,
        &format!("/api/v1/agents?limit=3&cursor={cursor}"),
    )
    .await;
    let mut all = names(&first, "hostname");
    all.extend(names(&rest, "hostname"));
    all.sort();
    assert_eq!(all, ["db-01", "old-01", "web-01", "win-01"]);

    // A cursor takes no offset and no other order; an unknown state is refused.
    for uri in [
        format!("/api/v1/agents?cursor={cursor}&offset=2"),
        format!("/api/v1/agents?cursor={cursor}&sort=hostname"),
        "/api/v1/agents?state=sleeping".to_owned(),
        "/api/v1/agents?sort=name".to_owned(),
    ] {
        let (status, _, _) = send(&app, request("GET", &uri, Some(&admin), None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_the_filtered_lists_and_their_counts_hold_only_the_rows_of_the_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "list-mine").await;
    let other = create_tenant_with_key(db.database(), "list-other").await;
    seed_hosts(&db, tenant).await;
    let [stranger, ..] = seed_hosts(&db, other).await;
    let secret = seed_secret(db.database(), other, "pw").await;
    let repository = seed_repository(db.database(), other, "other-repo", secret).await;
    seed_job(db.database(), other, stranger, repository).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let page = get(&app, &viewer, "/api/v1/agents?facets=true").await;
    assert_eq!(page["total"], 4);
    assert_eq!(page["facets"]["states"]["online"], 1);
    let linux = page["facets"]["os"]
        .as_array()
        .expect("the systems")
        .iter()
        .find(|item| item["value"] == "linux")
        .map(|item| item["count"].clone());
    assert_eq!(linux, Some(Value::from(3)));

    // A filter on a host of another tenant finds nothing.
    let page = get(&app, &viewer, &format!("/api/v1/jobs?agent_id={stranger}")).await;
    assert_eq!(page["total"], 0);
    let page = get(&app, &viewer, "/api/v1/jobs").await;
    assert_eq!(page["total"], 0);

    db.cleanup().await;
}

/// Three jobs: a running backup of `web-01`, a failed check of `db-01` from yesterday and a test of
/// access of `web-01` from last week.
async fn seed_jobs(db: &TestDatabase, tenant: TenantId, hosts: &[AgentId; 4]) -> [JobId; 3] {
    let secret = seed_secret(db.database(), tenant, "pw").await;
    // A repository holds one active job, so each job gets its own repository.
    let mut repositories = Vec::new();
    for name in ["repo-a", "repo-b", "repo-c"] {
        repositories.push(seed_repository(db.database(), tenant, name, secret).await);
    }
    let backup = seed_job(db.database(), tenant, hosts[0], repositories[0]).await;
    let check = seed_job(db.database(), tenant, hosts[1], repositories[1]).await;
    let old = seed_job(db.database(), tenant, hosts[0], repositories[2]).await;
    update(
        db,
        tenant,
        "UPDATE jobs SET kind = 'backup', status = 'running', started_at = now() WHERE id = $1",
        backup.as_uuid(),
    )
    .await;
    update(
        db,
        tenant,
        "UPDATE jobs SET kind = 'check', status = 'failed', \
         created_at = now() - interval '1 day', started_at = now() - interval '1 day', \
         finished_at = now() - interval '23 hours' WHERE id = $1",
        check.as_uuid(),
    )
    .await;
    update(
        db,
        tenant,
        "UPDATE jobs SET created_at = now() - interval '7 days' WHERE id = $1",
        old.as_uuid(),
    )
    .await;
    [backup, check, old]
}

fn ids(page: &Value) -> Vec<String> {
    names(page, "id")
}

#[tokio::test]
async fn fr_sc_09_the_job_list_filters_by_host_kind_status_and_period_and_pages_by_number() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "job-filters").await;
    let hosts = seed_hosts(&db, tenant).await;
    let [backup, check, old] = seed_jobs(&db, tenant, &hosts).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let page = get(&app, &viewer, "/api/v1/jobs").await;
    assert_eq!(ids(&page), [backup, check, old].map(|id| id.to_string()));
    assert_eq!(page["total"], 3);

    let page = get(
        &app,
        &viewer,
        &format!("/api/v1/jobs?agent_id={},{}", hosts[1], hosts[2]),
    )
    .await;
    assert_eq!(ids(&page), [check.to_string()]);
    let page = get(&app, &viewer, "/api/v1/jobs?kind=backup,check").await;
    assert_eq!(page["total"], 2);
    let page = get(&app, &viewer, "/api/v1/jobs?status=failed").await;
    assert_eq!(ids(&page), [check.to_string()]);

    let from = (OffsetDateTime::now_utc() - time::Duration::days(2))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("a time");
    let page = get(&app, &viewer, &format!("/api/v1/jobs?from={from}")).await;
    assert_eq!(ids(&page), [backup, check].map(|id| id.to_string()));

    // A numbered page in another order, and the total of all pages.
    let page = get(
        &app,
        &viewer,
        "/api/v1/jobs?sort=created_at&order=asc&limit=1&offset=1",
    )
    .await;
    assert_eq!(ids(&page), [check.to_string()]);
    assert_eq!(page["total"], 3);
    assert!(page["next_cursor"].is_null());
    let page = get(&app, &viewer, "/api/v1/jobs?sort=host&order=asc&limit=1").await;
    assert_eq!(ids(&page), [check.to_string()]);

    for uri in [
        "/api/v1/jobs?kind=nap",
        "/api/v1/jobs?from=yesterday",
        "/api/v1/jobs?agent_id=x",
    ] {
        let (status, _, _) = send(&app, request("GET", uri, Some(&viewer), None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }

    db.cleanup().await;
}
