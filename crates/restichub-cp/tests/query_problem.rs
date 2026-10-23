//! Every route that documents query parameters answers a value that does not parse with an RFC 9457
//! problem body, so a client that the document generates reads a code and a field name (ADR-0023).
#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::Value;

use common::{
    app, body_json, create_tenant_with_key, request, seed_agent, seed_job, seed_plan, seed_profile,
    seed_repository, seed_secret, seed_session, send_response,
};

/// Sends a request and returns the status, the content type and the body.
async fn send_problem(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = send_response(app, request).await;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = body_json(response).await;
    (status, content_type, body)
}

#[tokio::test]
async fn adr_0023_a_bad_query_parameter_gives_a_problem_on_every_route() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "query-problem").await;
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let platform_admin = seed_session(
        db.database(),
        restichub_db::PLATFORM_TENANT_ID,
        "root",
        "platform_admin",
    )
    .await;
    let password = seed_secret(db.database(), tenant, "repository-password").await;
    let repository = seed_repository(db.database(), tenant, "main", password).await;
    let agent = seed_agent(db.database(), tenant, "host-1").await;
    let profile = seed_profile(db.database(), tenant, "nightly").await;
    let plan = seed_plan(db.database(), tenant, "nightly", profile, agent, repository).await;
    let job = seed_job(db.database(), tenant, agent, repository).await;
    let app = app(db.database());

    // Each case names the route, the cookie, the query and the field that the problem reports. An
    // unknown parameter reports the parameter `query`, so the response never echoes the request.
    // Every route that refuses an unknown parameter has a `colour` case: a raw `Query` extractor
    // answers such a request with `text/plain`, so the case fails on that code.
    let cases = [
        (
            "/api/v1/enrollment-tokens?limit=x".to_owned(),
            &admin,
            "limit",
        ),
        (
            "/api/v1/enrollment-tokens?colour=red".to_owned(),
            &admin,
            "query",
        ),
        (
            "/api/v1/platform/tenants?limit=x".to_owned(),
            &platform_admin,
            "limit",
        ),
        (
            format!("/api/v1/repositories/{repository}/snapshots?from=yesterday"),
            &admin,
            "from",
        ),
        (
            format!("/api/v1/repositories/{repository}/snapshots?colour=red"),
            &admin,
            "query",
        ),
        (
            format!("/api/v1/repositories/{repository}/ls?job={job}&colour=red"),
            &admin,
            "query",
        ),
        ("/api/v1/profiles?colour=red".to_owned(), &admin, "query"),
        ("/api/v1/plans?limit=x".to_owned(), &admin, "limit"),
        ("/api/v1/plans?colour=red".to_owned(), &admin, "query"),
        (
            format!("/api/v1/plans/{plan}/next-runs?count=x"),
            &admin,
            "count",
        ),
        (
            format!("/api/v1/plans/{plan}/next-runs?colour=red"),
            &admin,
            "query",
        ),
        (format!("/api/v1/jobs/{job}/logs?limit=x"), &admin, "limit"),
        (
            format!("/api/v1/jobs/{job}/logs?colour=red"),
            &admin,
            "query",
        ),
        ("/api/v1/jobs?colour=red".to_owned(), &admin, "query"),
        ("/api/v1/agents?colour=red".to_owned(), &admin, "query"),
        ("/api/v1/alerts?colour=red".to_owned(), &admin, "query"),
        ("/api/v1/audit?colour=red".to_owned(), &admin, "query"),
        (
            "/api/v1/platform/audit?colour=red".to_owned(),
            &platform_admin,
            "query",
        ),
    ];
    for (uri, cookie, field) in cases {
        let (status, content_type, body) =
            send_problem(&app, request("GET", &uri, Some(cookie), None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert_eq!(
            content_type.as_deref(),
            Some("application/problem+json"),
            "{uri}: {body}"
        );
        assert_eq!(body["code"], "invalid_argument", "{uri}: {body}");
        assert_eq!(body["field"], field, "{uri}: {body}");
    }

    db.cleanup().await;
}
