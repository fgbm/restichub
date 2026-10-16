//! Numbered pages of the alert list: an offset, the kind and search filters, the total and its
//! bounds (FR-UI-01, FR-MN-03). The list stays inside its tenant (ADR-0006).
#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::TenantId;
use restichub_db::test_util::TestDatabase;
use serde_json::Value;

use common::{app, create_tenant, request, seed_session, send};

/// Writes `count` resolved alerts of `kind` for the host `hostname`, one second apart.
async fn seed_alerts(db: &TestDatabase, tenant: TenantId, kind: &str, hostname: &str, count: i64) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(
        "INSERT INTO alert_state (id, tenant_id, kind, dedupe_key, status, agent_hostname, \
                                  opened_at, resolved_at) \
         SELECT gen_random_uuid(), $1, $2, $3 || n, 'resolved', $3, \
                now() - n * interval '1 second', now() \
           FROM generate_series(1, $4) AS n",
    )
    .bind(tenant.as_uuid())
    .bind(kind)
    .bind(hostname)
    .bind(count)
    .execute(&mut *tx)
    .await
    .expect("cannot write the alerts");
    tx.commit().await.expect("cannot commit the alerts");
}

fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("the page holds items")
        .iter()
        .map(|item| item["id"].as_str().expect("an alert has an id").to_owned())
        .collect()
}

#[tokio::test]
async fn fr_ui_01_the_alert_list_reads_numbered_pages_with_filters() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "alert-pages").await;
    let other = create_tenant(db.database(), "alert-pages-other").await;
    seed_alerts(&db, tenant, "job_failed", "web-01", 5).await;
    seed_alerts(&db, tenant, "agent_offline", "db_01", 3).await;
    seed_alerts(&db, other, "job_failed", "web-01", 4).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());
    let get = |query: String| {
        request(
            "GET",
            &format!("/api/v1/alerts?{query}"),
            Some(&viewer),
            None,
        )
    };

    let (status, _, first) = send(&app, get("status=all&limit=3".to_owned())).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["total"], 8);
    assert_eq!(first["total_capped"], false);
    assert!(first["next_cursor"].is_string());

    let mut seen = Vec::new();
    for offset in [0, 3, 6] {
        let (status, _, page) =
            send(&app, get(format!("status=all&limit=3&offset={offset}"))).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["total"], 8);
        assert!(
            page["next_cursor"].is_null(),
            "a page read by offset has no cursor"
        );
        seen.extend(ids(&page));
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        (seen.len(), unique.len()),
        (8, 8),
        "the pages repeat or miss an alert"
    );

    let (_, _, page) = send(&app, get("status=all&kind=agent_offline".to_owned())).await;
    assert_eq!(page["total"], 3);
    let (_, _, page) = send(
        &app,
        get("status=all&kind=agent_offline,job_failed".to_owned()),
    )
    .await;
    assert_eq!(page["total"], 8);
    let (_, _, page) = send(&app, get("status=all&q=WEB".to_owned())).await;
    assert_eq!(page["total"], 5);
    // `_` is a literal character, not a wildcard of one character.
    let (_, _, page) = send(&app, get("status=all&q=db_".to_owned())).await;
    assert_eq!(page["total"], 3);
    let (_, _, page) = send(&app, get("status=all&q=web_".to_owned())).await;
    assert_eq!(page["total"], 0);
    let (_, _, page) = send(&app, get("status=open".to_owned())).await;
    assert_eq!(page["total"], 0);

    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_an_alert_parameter_out_of_bounds_gives_a_stable_code() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "alert-bounds").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let cases = [
        ("offset=10001".to_owned(), "offset", "out_of_range"),
        ("offset=-1".to_owned(), "offset", "out_of_range"),
        (
            "offset=0&cursor=1;00000000-0000-0000-0000-000000000000".to_owned(),
            "offset",
            "invalid_format",
        ),
        ("kind=job_failed,sleeping".to_owned(), "kind", "unknown"),
        (format!("q={}", "a".repeat(201)), "q", "too_long"),
    ];
    for (query, field, reason) in cases {
        let (status, _, problem) = send(
            &app,
            request(
                "GET",
                &format!("/api/v1/alerts?{query}"),
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
    let (status, _, page) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/alerts?q={}&offset=10000", "a".repeat(200)),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");

    db.cleanup().await;
}

#[tokio::test]
async fn fr_ui_01_the_alert_list_counts_a_long_history_only_up_to_the_limit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "alert-count").await;
    seed_alerts(&db, tenant, "job_failed", "web-01", 10_001).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let (status, _, page) = send(
        &app,
        request(
            "GET",
            "/api/v1/alerts?status=resolved&limit=1",
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
            "/api/v1/alerts?status=resolved&offset=10000&limit=5",
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(ids(&page).len(), 1);

    db.cleanup().await;
}
