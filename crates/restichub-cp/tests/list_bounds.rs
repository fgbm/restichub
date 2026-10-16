//! The bounds of the filtered lists: each parameter that is out of bounds or does not parse gives a
//! problem body with a stable code, and a long history is counted only up to a limit (FR-UI-01).
#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::TenantId;
use restichub_db::test_util::TestDatabase;

use common::{
    app, create_tenant_with_key, request, seed_agent, seed_job, seed_repository, seed_secret,
    seed_session, send,
};

#[tokio::test]
async fn fr_ui_01_a_list_parameter_out_of_bounds_gives_a_stable_code() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "list-bounds").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let long_search = "a".repeat(201);
    let many_systems = vec!["linux"; 51].join(",");
    let many_ids = vec![uuid::Uuid::nil().to_string(); 51].join(",");
    let cases = [
        (format!("/api/v1/agents?q={long_search}"), "q", "too_long"),
        (
            format!("/api/v1/agents?os={many_systems}"),
            "os",
            "too_long",
        ),
        (
            format!("/api/v1/jobs?plan_id={many_ids}"),
            "plan_id",
            "too_long",
        ),
        (
            "/api/v1/agents?offset=10001".to_owned(),
            "offset",
            "out_of_range",
        ),
        (
            "/api/v1/jobs?offset=10001".to_owned(),
            "offset",
            "out_of_range",
        ),
        (
            "/api/v1/agents?offset=-1".to_owned(),
            "offset",
            "out_of_range",
        ),
        (
            "/api/v1/jobs?offset=-1".to_owned(),
            "offset",
            "out_of_range",
        ),
        (
            "/api/v1/jobs?from=2026-10-02T00:00:00Z&to=2026-10-01T00:00:00Z".to_owned(),
            "to",
            "out_of_range",
        ),
        (
            "/api/v1/agents?sort=name".to_owned(),
            "sort",
            "invalid_format",
        ),
        (
            "/api/v1/jobs?sort=name".to_owned(),
            "sort",
            "invalid_format",
        ),
        (
            "/api/v1/agents?order=up".to_owned(),
            "order",
            "invalid_format",
        ),
        (
            "/api/v1/agents?server_scripts=maybe".to_owned(),
            "server_scripts",
            "invalid_format",
        ),
        (
            "/api/v1/agents?facets=maybe".to_owned(),
            "facets",
            "invalid_format",
        ),
        (
            "/api/v1/agents?status=sleeping".to_owned(),
            "status",
            "invalid_format",
        ),
        (
            "/api/v1/jobs?status=nap".to_owned(),
            "status",
            "invalid_format",
        ),
        (
            "/api/v1/agents?offset=x".to_owned(),
            "offset",
            "invalid_format",
        ),
        ("/api/v1/agents?colour=red".to_owned(), "query", "unknown"),
    ];
    for (uri, field, reason) in cases {
        let (status, _, body) = send(&app, request("GET", &uri, Some(&viewer), None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["code"], "invalid_argument", "{uri}: {body}");
        assert_eq!(body["field"], field, "{uri}: {body}");
        assert_eq!(body["reason"], reason, "{uri}: {body}");
    }

    let search = "a".repeat(200);
    let systems = vec!["linux"; 50].join(",");
    for uri in [
        format!("/api/v1/agents?q={search}&os={systems}&offset=10000"),
        "/api/v1/jobs?offset=10000&from=2026-10-01T00:00:00Z&to=2026-10-01T00:00:00Z".to_owned(),
        "/api/v1/agents?status=active".to_owned(),
    ] {
        let (status, _, body) = send(&app, request("GET", &uri, Some(&viewer), None)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }

    db.cleanup().await;
}

/// Copies one job of the tenant `copies` times with new identifiers.
async fn copy_job(db: &TestDatabase, tenant: TenantId, job: uuid::Uuid, copies: i64) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let statement = format!(
        "DO $$ DECLARE cols text; vals text; BEGIN \
           SELECT string_agg(quote_ident(column_name), ', ' ORDER BY ordinal_position), \
                  string_agg(CASE WHEN column_name = 'id' THEN 'gen_random_uuid()' \
                             ELSE quote_ident(column_name) END, ', ' ORDER BY ordinal_position) \
             INTO cols, vals FROM information_schema.columns \
            WHERE table_schema = current_schema() AND table_name = 'jobs'; \
           EXECUTE format('INSERT INTO jobs (%s) SELECT %s FROM jobs \
                           CROSS JOIN generate_series(1, {copies}) WHERE id = %L', \
                          cols, vals, '{job}'); \
         END $$"
    );
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(&mut *tx)
        .await
        .expect("cannot copy the job");
    tx.commit().await.expect("cannot commit the copies");
}

#[tokio::test]
async fn fr_ui_01_the_job_list_counts_a_long_history_only_up_to_the_limit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "list-count").await;
    let agent = seed_agent(db.database(), tenant, "web-01").await;
    let secret = seed_secret(db.database(), tenant, "pw").await;
    let repository = seed_repository(db.database(), tenant, "repo", secret).await;
    let job = seed_job(db.database(), tenant, agent, repository).await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let (status, _, page) = send(&app, request("GET", "/api/v1/jobs", Some(&viewer), None)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 1);
    assert_eq!(page["total_capped"], false);

    // A repository holds one active job, so the copies come from a finished one.
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(
        "UPDATE jobs SET status = 'succeeded', result_code = 'succeeded', started_at = now(), \
         finished_at = now() WHERE id = $1",
    )
    .bind(job.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("cannot finish the job");
    tx.commit().await.expect("cannot commit the update");
    copy_job(&db, tenant, job.as_uuid(), 10_000).await;

    let (status, _, page) = send(
        &app,
        request("GET", "/api/v1/jobs?limit=1", Some(&viewer), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 10_000);
    assert_eq!(page["total_capped"], true);
    let (status, _, page) = send(
        &app,
        request(
            "GET",
            "/api/v1/jobs?offset=10000&limit=5",
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["items"].as_array().map(Vec::len), Some(1));

    db.cleanup().await;
}
