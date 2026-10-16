//! Numbered pages of the audit log: an offset, the total of the filter and its bounds (FR-UI-01,
//! FR-AU-01). The tenant list stays inside its tenant (ADR-0006).
#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::TenantId;
use restichub_db::test_util::TestDatabase;
use serde_json::Value;

use common::{app, create_platform_admin, create_tenant, request, seed_session, send};

const FILTER: &str = "action=job_run";

/// Writes `count` records of the action `job_run` one millisecond apart.
async fn seed_records(db: &TestDatabase, tenant: TenantId, count: i64) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, occurred_at, actor_type, action, object_type) \
         SELECT gen_random_uuid(), $1, now() - n * interval '1 millisecond', 'system', \
                'job_run', 'job' \
           FROM generate_series(1, $2) AS n",
    )
    .bind(tenant.as_uuid())
    .bind(count)
    .execute(&mut *tx)
    .await
    .expect("cannot write the audit records");
    tx.commit().await.expect("cannot commit the records");
}

fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("the page holds items")
        .iter()
        .map(|item| item["id"].as_str().expect("an item has an id").to_owned())
        .collect()
}

#[tokio::test]
async fn fr_ui_01_the_audit_log_reads_numbered_pages_with_a_total() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "audit-pages").await;
    let other = create_tenant(db.database(), "audit-pages-other").await;
    seed_records(&db, tenant, 7).await;
    seed_records(&db, other, 4).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let uri = |query: &str| format!("/api/v1/audit?{FILTER}&limit=3{query}");
    let (status, _, first) = send(&app, request("GET", &uri(""), Some(&viewer), None)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["total"], 7);
    assert_eq!(first["total_capped"], false);
    assert!(first["next_cursor"].is_string());

    let mut seen = Vec::new();
    for offset in [0, 3, 6] {
        let (status, _, page) = send(
            &app,
            request(
                "GET",
                &uri(&format!("&offset={offset}")),
                Some(&viewer),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["total"], 7, "the total ignores the offset");
        assert!(
            page["next_cursor"].is_null(),
            "a page read by offset has no cursor"
        );
        seen.extend(ids(&page));
    }
    assert_eq!(seen.len(), 7);
    assert_eq!(seen[..3], ids(&first)[..]);
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 7, "the pages repeat a record");

    let (status, _, past) =
        send(&app, request("GET", &uri("&offset=9"), Some(&viewer), None)).await;
    assert_eq!(status, StatusCode::OK, "{past}");
    assert!(ids(&past).is_empty());
    assert_eq!(past["total"], 7);

    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_platform_audit_log_counts_every_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let first = create_tenant(db.database(), "audit-pages-a").await;
    let second = create_tenant(db.database(), "audit-pages-b").await;
    seed_records(&db, first, 3).await;
    seed_records(&db, second, 2).await;
    let admin = create_platform_admin(db.database(), "platform", "phc").await;
    let cookie =
        common::session_cookie(db.database(), restichub_db::PLATFORM_TENANT_ID, admin).await;
    let app = app(db.database());

    let (status, _, page) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/platform/audit?{FILTER}&limit=2&offset=2"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 5);
    assert_eq!(ids(&page).len(), 2);
    let (status, _, page) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/platform/audit?{FILTER}&tenant_id={second}&offset=0"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 2);

    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_an_audit_offset_out_of_bounds_gives_a_stable_code() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "audit-bounds").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let (_, _, page) = send(
        &app,
        request("GET", "/api/v1/audit?limit=1", Some(&viewer), None),
    )
    .await;
    let cursor = page["next_cursor"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| "1;00000000-0000-0000-0000-000000000000".to_owned());
    let cases = [
        ("offset=10001".to_owned(), "offset", "out_of_range"),
        ("offset=-1".to_owned(), "offset", "out_of_range"),
        ("offset=many".to_owned(), "offset", "invalid_format"),
        ("limit=x".to_owned(), "limit", "invalid_format"),
        (
            format!("offset=0&cursor={cursor}"),
            "offset",
            "invalid_format",
        ),
    ];
    for (query, field, reason) in cases {
        let (status, _, problem) = send(
            &app,
            request(
                "GET",
                &format!("/api/v1/audit?{query}"),
                Some(&viewer),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {problem}");
        assert_eq!(problem["code"], "invalid_argument", "{query}");
        assert_eq!(problem["field"], field, "{query}: {problem}");
        assert_eq!(problem["reason"], reason, "{query}: {problem}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_audit_log_counts_a_long_log_only_up_to_the_limit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "audit-count").await;
    seed_records(&db, tenant, 10_001).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let (status, _, page) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/audit?{FILTER}&limit=1"),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 10_000);
    assert_eq!(page["total_capped"], true);
    let (status, _, page) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/audit?{FILTER}&offset=10000&limit=5"),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(ids(&page).len(), 1);

    db.cleanup().await;
}
