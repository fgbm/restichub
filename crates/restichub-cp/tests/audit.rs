//! Audit API and audit service tests (AC-AU-01, AC-RP-03).
//!
//! A test builds its authentication through a real session in the database. The extractor no longer
//! accepts a context from the request extensions.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use restichub_cp::audit::AuditService;
use restichub_cp::config::TenancyConfig;
use restichub_cp::server::{AppState, router};
use restichub_db::audit::{self, Actor, AuditAction, AuditEvent, AuditFilter, AuditPage};
use restichub_db::test_util::{TestDatabase, test_database};
use restichub_db::{
    Database, DatabaseConfig, PLATFORM_TENANT_ID, SecretId, TenantId, sessions, tenants, users,
};
use serde_json::Value;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

/// The plaintext marker that must never appear in an audit row.
const CANARY: &str = "canary-secret-value";

const AUDIT_TENANT_PATH: &str = "/api/v1/audit";
const AUDIT_PLATFORM_PATH: &str = "/api/v1/platform/audit";

fn peer() -> ConnectInfo<std::net::SocketAddr> {
    ConnectInfo(
        "203.0.113.7:1234"
            .parse()
            .unwrap_or_else(|_| std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0))),
    )
}

/// Builds a router with a lazy database. The database points to no server.
fn lazy_app() -> Result<Router, Box<dyn std::error::Error>> {
    let config = DatabaseConfig {
        url: "postgres://restichub@127.0.0.1:1/restichub".to_owned(),
        max_connections: 1,
        acquire_timeout: Duration::from_millis(100),
    };
    let database = Database::connect_lazy(&config, None)?;
    // The platform audit path is under test, so the router runs in multi mode.
    Ok(router(
        AppState::new(database).with_tenancy(TenancyConfig::multi()),
    ))
}

async fn body_json(
    response: axum::response::Response,
) -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok(serde_json::from_slice(&bytes)?)
}

/// Builds a request with the socket peer and an optional session cookie.
fn request(
    method: &str,
    uri: &str,
    cookie: Option<&str>,
) -> Result<Request<Body>, Box<dyn std::error::Error>> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    let mut request = builder.body(Body::empty())?;
    request.extensions_mut().insert(peer());
    Ok(request)
}

/// Creates a session for a user with a role in a tenant and returns the cookie header value.
async fn sign_in(
    db: &TestDatabase,
    tenant: TenantId,
    role: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut tx = db.database().begin_tenant(tenant).await?;
    let user = users::create_member(
        &mut tx,
        &users::NewUser {
            username: "tester",
            password_hash: "phc",
            display_name: "Tester",
        },
        restichub_db::Role::parse(role).ok_or("the role is known")?,
    )
    .await?;
    let value = restichub_cp::token::random_token()?;
    let hash = restichub_cp::token::hash_token(&value);
    let now = OffsetDateTime::now_utc();
    sessions::create(
        &mut tx,
        user,
        &hash,
        now + time::Duration::hours(12),
        now,
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(format!("__Host-restichub_session={value}"))
}

#[tokio::test]
async fn ac_au_01_api_has_no_audit_mutation_endpoint() -> Result<(), Box<dyn std::error::Error>> {
    let response = lazy_app()?
        .oneshot(
            Request::builder()
                .uri("/api/v1/openapi.json")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let document = body_json(response).await?;
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .ok_or("the document holds no paths")?;
    // The audit paths of the router and of the document are the same.
    for path in [AUDIT_TENANT_PATH, AUDIT_PLATFORM_PATH] {
        let item = paths.get(path).ok_or("the document holds no audit path")?;
        let methods = item.as_object().ok_or("a path item is not an object")?;
        for method in methods.keys() {
            assert_eq!(
                method, "get",
                "the audit path {path} holds a non-GET method {method}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn ac_au_01_audit_routes_refuse_a_mutation() -> Result<(), Box<dyn std::error::Error>> {
    for path in [AUDIT_TENANT_PATH, AUDIT_PLATFORM_PATH] {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let response = lazy_app()?
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path} must not change an audit record"
            );
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            assert_eq!(
                content_type.as_deref(),
                Some("application/problem+json"),
                "{method} {path} must return a problem"
            );
            let problem = body_json(response).await?;
            assert_eq!(
                problem.get("code").and_then(Value::as_str),
                Some("method_not_allowed")
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn the_audit_endpoint_rejects_an_unauthenticated_request()
-> Result<(), Box<dyn std::error::Error>> {
    let response = lazy_app()?
        .oneshot(
            Request::builder()
                .uri(AUDIT_TENANT_PATH)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    assert_eq!(content_type.as_deref(), Some("application/problem+json"));
    let problem = body_json(response).await?;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("unauthenticated")
    );
    Ok(())
}

#[tokio::test]
async fn the_platform_audit_endpoint_refuses_a_tenant_role()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let mut tx = db.database().begin_platform().await?;
    let tenant = tenants::create(&mut tx, "audit-forbidden").await?;
    tx.commit().await?;
    let cookie = sign_in(&db, tenant, "admin").await?;

    let app = router(AppState::new(db.database().clone()).with_tenancy(TenancyConfig::multi()));
    let response = app
        .oneshot(request("GET", AUDIT_PLATFORM_PATH, Some(&cookie))?)
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let problem = body_json(response).await?;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("forbidden")
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn the_audit_endpoint_refuses_a_bad_query() -> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let mut tx = db.database().begin_platform().await?;
    let tenant = tenants::create(&mut tx, "audit-bad-query").await?;
    tx.commit().await?;
    let cookie = sign_in(&db, tenant, "viewer").await?;

    let app = router(AppState::new(db.database().clone()).with_tenancy(TenancyConfig::multi()));
    let response = app
        .oneshot(request(
            "GET",
            &format!("{AUDIT_TENANT_PATH}?limit=abc"),
            Some(&cookie),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let problem = body_json(response).await?;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("invalid_argument")
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_03_secret_delivery_record_has_required_fields_and_no_canary()
-> Result<(), Box<dyn std::error::Error>> {
    // This test proves that the helper writes the tenant, the agent, the job, the repository and
    // the time, and that the stored record holds no canary. The helper takes no secret value. The
    // full proof that no secret value appears happens on the real delivery path in epic #1/#2.
    let Some(db) = test_database().await else {
        return Ok(());
    };

    let mut tx = db.database().begin_platform().await?;
    let tenant = tenants::create(&mut tx, "audit-tenant").await?;
    tx.commit().await?;

    let agent_id = Uuid::now_v7();
    let job_id = Uuid::now_v7();
    let repository_id = Uuid::now_v7();
    let secret_id = SecretId::new();

    let mut tx = db.database().begin_tenant(tenant).await?;
    AuditService::record_secret_delivered(&mut tx, agent_id, job_id, repository_id, secret_id)
        .await?;
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant).await?;
    let result = audit::list(&mut tx, &AuditFilter::default(), &AuditPage::default()).await?;
    tx.commit().await?;
    let record = result
        .records
        .iter()
        .find(|record| record.action == AuditAction::SecretDelivered)
        .ok_or("the delivery record is missing")?;
    assert_eq!(record.tenant_id, tenant);
    assert_eq!(record.object_type, "secret");
    assert_eq!(record.object_id, Some(secret_id.as_uuid()));
    assert!(record.occurred_at.unix_timestamp() > 0);
    let diff = record.diff.as_ref().ok_or("the record holds no diff")?;
    let text = diff.to_string();
    assert!(text.contains(&agent_id.to_string()), "{text}");
    assert!(text.contains(&job_id.to_string()), "{text}");
    assert!(text.contains(&repository_id.to_string()), "{text}");
    assert!(!text.contains(CANARY), "{text}");
    let row = format!("{record:?}");
    assert!(!row.contains(CANARY), "{row}");

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn a_viewer_reads_the_tenant_audit_records() -> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let mut tx = db.database().begin_platform().await?;
    let tenant = tenants::create(&mut tx, "audit-read").await?;
    tx.commit().await?;

    let mut event = AuditEvent::new(AuditAction::ConfigUpdate, "repository");
    event.actor = Actor::System;
    let mut tx = db.database().begin_tenant(tenant).await?;
    audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    let cookie = sign_in(&db, tenant, "viewer").await?;

    let app = router(AppState::new(db.database().clone()).with_tenancy(TenancyConfig::multi()));
    let response = app
        .oneshot(request("GET", AUDIT_TENANT_PATH, Some(&cookie))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let page = body_json(response).await?;
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .ok_or("no items")?;
    assert_eq!(items.len(), 1);
    assert_eq!(
        items
            .first()
            .and_then(|item| item.get("action"))
            .and_then(Value::as_str),
        Some("config_update")
    );
    assert!(page.get("next_cursor").is_some_and(Value::is_null));

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn the_platform_audit_endpoint_returns_every_tenant() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let mut tx = db.database().begin_platform().await?;
    let tenant = tenants::create(&mut tx, "audit-platform").await?;
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant).await?;
    audit::insert(
        &mut tx,
        &AuditEvent::new(AuditAction::AgentEnrolled, "agent"),
    )
    .await?;
    tx.commit().await?;
    let cookie = sign_in(&db, PLATFORM_TENANT_ID, "platform_admin").await?;

    let app = router(AppState::new(db.database().clone()).with_tenancy(TenancyConfig::multi()));
    let response = app
        .oneshot(request("GET", AUDIT_PLATFORM_PATH, Some(&cookie))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let page = body_json(response).await?;
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .ok_or("no items")?;
    assert!(items.iter().any(|item| {
        item.get("tenant_id").and_then(Value::as_str) == Some(tenant.to_string().as_str())
    }));

    db.cleanup().await;
    Ok(())
}
