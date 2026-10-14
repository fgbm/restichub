//! The enrollment exchange (ADR-0026, FR-ID-01, FR-ID-02, FR-ID-05, FR-TN-03).
//!
//! The tests cover AC-ID-01, AC-ID-05, the refusal cases of AC-IN-02, the certificate identity,
//! the lifetime, the request policy and the concurrent use of one token.

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use prost::Message as _;
use rcgen::{CertificateParams, DnType, KeyPair, PKCS_ECDSA_P256_SHA256, PKCS_ECDSA_P384_SHA384};
use restichub_db::enrollment_tokens;
use restichub_proto::agent::v1::{KeyUpdate, SigningKeyUpdate};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

use common::{
    AGENT_ENDPOINT, agent_router, api_router, ca_without_signing_state, count, create_tenant,
    create_user, pki_state, request, seed_enrollment_token_value, seed_session, send,
};

/// Builds the web router and the agent listener router over one state (ADR-0026).
async fn routers(db: &restichub_db::Database) -> (Router, Router) {
    let (state, _ca) = pki_state(db).await;
    (api_router(state.clone()), agent_router(state))
}

/// Builds a certificate signing request with an ECDSA P-256 key (ADR-0026).
fn p256_csr() -> Vec<u8> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    CertificateParams::default()
        .serialize_request(&key)
        .expect("cannot build the CSR")
        .der()
        .to_vec()
}

/// Builds a request with a chosen subject, so a test can prove that the control plane ignores it.
fn csr_with_subject(common_name: &str) -> Vec<u8> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    params
        .serialize_request(&key)
        .expect("cannot build the CSR")
        .der()
        .to_vec()
}

fn body(token: &str, csr: &[u8]) -> Value {
    json!({
        "token": token,
        "csr": STANDARD.encode(csr),
        "hostname": "host-a",
        "os": "linux",
        "arch": "amd64",
        "agent_version": "0.1.0",
        "restic_version": "0.19.0",
    })
}

async fn enroll(app: &Router, token: &str, csr: &[u8]) -> (StatusCode, Value) {
    let (status, _, response) = send(
        app,
        request("POST", "/agent/v1/enroll", None, Some(body(token, csr))),
    )
    .await;
    (status, response)
}

fn decode_cert(base64: &str) -> Vec<u8> {
    STANDARD.decode(base64).expect("the certificate is base64")
}

fn parse(der: &[u8]) -> X509Certificate<'_> {
    X509Certificate::from_der(der)
        .expect("cannot parse the certificate")
        .1
}

/// The one refusal body the client sees, without the request ID.
fn refusal_signature(problem: &Value) -> (u16, String, String, String) {
    (
        problem.get("status").and_then(Value::as_u64).unwrap_or(0) as u16,
        problem
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        problem
            .get("field")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        problem
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    )
}

async fn agents_of(app: &Router, cookie: &str) -> Vec<Value> {
    let (status, _, page) = send(app, request("GET", "/api/v1/agents", Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK);
    page.get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

async fn audit_items(app: &Router, cookie: &str, action: &str) -> Vec<Value> {
    let (status, _, page) = send(
        app,
        request(
            "GET",
            &format!("/api/v1/audit?action={action}"),
            Some(cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    page.get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// AC-ID-01: a valid token of tenant A enrolls one agent; the certificate carries the identity; the
/// agent shows only in tenant A.
#[tokio::test]
async fn ac_id_01_a_valid_token_enrolls_an_agent_in_its_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant(db.database(), "enroll-a").await;
    let tenant_b = create_tenant(db.database(), "enroll-b").await;
    let creator = create_user(db.database(), tenant_a, "enroll-admin", "phc", "admin").await;
    let labels = vec!["host".to_owned(), "linux".to_owned()];
    let (token, _id) = seed_enrollment_token_value(
        db.database(),
        tenant_a,
        creator,
        &labels,
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let (app, agent) = routers(db.database()).await;

    let (status, response) = enroll(&agent, &token, &p256_csr()).await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    let agent_id = response
        .get("agent_id")
        .and_then(Value::as_str)
        .expect("the response holds the agent id")
        .to_owned();
    assert_eq!(
        response.get("tenant_id").and_then(Value::as_str),
        Some(tenant_a.to_string().as_str())
    );
    assert_eq!(
        response.get("grpc_endpoint").and_then(Value::as_str),
        Some(AGENT_ENDPOINT)
    );

    // The certificate carries the SAN URI and the agent UUID as the common name (ADR-0026).
    let cert_der = decode_cert(response["certificate"].as_str().expect("the certificate"));
    let cert = parse(&cert_der);
    let expected_uri = format!("restichub://tenant/{tenant_a}/agent/{agent_id}");
    let san = cert
        .subject_alternative_name()
        .expect("cannot read the SAN")
        .expect("the SAN is present");
    assert!(
        san.value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::URI(uri) if *uri == expected_uri.as_str())),
        "the SAN URI is missing: {:?}",
        san.value.general_names
    );
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .expect("the common name is present")
        .as_str()
        .expect("the common name is text");
    assert_eq!(cn, agent_id);

    // FR-ID-03: the certificate lifetime is 30 days.
    let days = (cert.validity().not_after.timestamp() - OffsetDateTime::now_utc().unix_timestamp())
        as f64
        / 86_400.0;
    assert!((days - 30.0).abs() < 0.2, "the lifetime is {days} days");

    // The chain is the issuing CA and the root, and the leaf chains to it.
    let chain = response["ca_chain"]
        .as_array()
        .expect("the response holds the chain");
    assert_eq!(chain.len(), 2);
    let issuing_der = decode_cert(chain[0].as_str().expect("the issuing CA"));
    let root_der = decode_cert(chain[1].as_str().expect("the root CA"));
    let issuing = parse(&issuing_der);
    let root = parse(&root_der);
    cert.verify_signature(Some(issuing.public_key()))
        .expect("the leaf must chain to the issuing CA");
    issuing
        .verify_signature(Some(root.public_key()))
        .expect("the issuing CA must chain to the root");

    // The response carries the signed job signing key set (FR-PR-04, FR-PR-11).
    let update_der = STANDARD
        .decode(
            response["signing_key_update"]
                .as_str()
                .expect("the key update"),
        )
        .expect("the key update is base64");
    let signed =
        SigningKeyUpdate::decode(update_der.as_slice()).expect("the key update must decode");
    assert!(!signed.signer_key_id.is_empty());
    let inner = KeyUpdate::decode(signed.key_update.as_slice()).expect("the inner update decodes");
    assert_eq!(inner.public_key.len(), 32);
    assert_eq!(inner.signer_key_id, signed.signer_key_id);

    // The agent shows in tenant A with the recorded facts and labels, and not in tenant B.
    let reader_a = seed_session(db.database(), tenant_a, "reader-a", "viewer").await;
    let reader_b = seed_session(db.database(), tenant_b, "reader-b", "viewer").await;
    let agents_a = agents_of(&app, &reader_a).await;
    assert_eq!(agents_a.len(), 1, "{agents_a:?}");
    assert_eq!(
        agents_a[0].get("id").and_then(Value::as_str),
        Some(agent_id.as_str())
    );
    assert_eq!(
        agents_a[0].get("restic_version").and_then(Value::as_str),
        Some("0.19.0")
    );
    assert_eq!(
        agents_a[0]
            .get("labels")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert!(agents_of(&app, &reader_b).await.is_empty());

    // The audit holds the use and the enroll in tenant A.
    assert_eq!(audit_items(&app, &reader_a, "agent_enroll").await.len(), 1);
    assert_eq!(
        audit_items(&app, &reader_a, "enrollment_token_use")
            .await
            .len(),
        1
    );

    db.cleanup().await;
}

/// AC-ID-01 and AC-IN-02: a used, an expired, a revoked and an unknown token give the same generic
/// refusal; only the audit record names the reason.
#[tokio::test]
async fn ac_in_02_a_used_expired_revoked_or_unknown_token_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-refuse").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (app, agent) = routers(db.database()).await;
    let reader = seed_session(db.database(), tenant, "reader", "viewer").await;

    // Used: the first enroll succeeds, the second gives the generic refusal.
    let (used, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    assert_eq!(
        enroll(&agent, &used, &p256_csr()).await.0,
        StatusCode::CREATED
    );
    let (status, used_refusal) = enroll(&agent, &used, &p256_csr()).await;

    // Expired: the row stays unused after the refusal.
    let (expired, expired_id) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() - Duration::seconds(1),
    )
    .await;
    let (_, expired_refusal) = enroll(&agent, &expired, &p256_csr()).await;

    // Revoked: an Admin revokes the unused token.
    let (revoked, revoked_id) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        enrollment_tokens::revoke(&mut tx, revoked_id, creator, OffsetDateTime::now_utc())
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    );
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    let (_, revoked_refusal) = enroll(&agent, &revoked, &p256_csr()).await;

    // Unknown: no token holds the hash.
    let (_, unknown_refusal) = enroll(&agent, "re_unknown-token-value", &p256_csr()).await;

    // One generic refusal for the client: the status, the code, the field and the reason match.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let reference = refusal_signature(&used_refusal);
    for problem in [&expired_refusal, &revoked_refusal, &unknown_refusal] {
        assert_eq!(refusal_signature(problem), reference, "{problem}");
    }

    // The refusal reason lives only in the audit record of the right tenant (used, expired,
    // revoked). The unknown token has no tenant, so it writes no record.
    let mut reasons: Vec<String> = audit_items(&app, &reader, "enrollment_token_refuse")
        .await
        .iter()
        .filter_map(|item| {
            item.get("diff")
                .and_then(|diff| diff.get("new"))
                .and_then(|new| new.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    reasons.sort();
    assert_eq!(reasons, vec!["expired", "revoked", "used"]);

    // The expired token is not consumed by the refusal.
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let entry = enrollment_tokens::get(&mut tx, expired_id)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("the expired token row is present");
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    assert!(entry.used_at.is_none(), "the refusal must not consume");
    assert!(entry.revoked_at.is_none());

    db.cleanup().await;
}

/// AC-ID-05: the 11th enrollment request from one address in one minute gives HTTP 429.
#[tokio::test]
async fn ac_id_05_the_eleventh_enrollment_from_one_ip_is_rate_limited() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (_app, agent) = routers(db.database()).await;
    let csr = p256_csr();
    for _ in 0..10 {
        let (status, _) = enroll(&agent, "re_unknown-token-value", &csr).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, problem) = enroll(&agent, "re_unknown-token-value", &csr).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("rate_limited")
    );

    db.cleanup().await;
}

/// AC-ID-01 supports and FR-PR-08: the request carries no tenant field, and the subject of the CSR
/// is ignored. A forged `tenant_id` is refused, and a forged subject does not reach the
/// certificate.
#[tokio::test]
async fn ac_tn_02_the_request_carries_no_tenant_and_the_subject_is_ignored() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-forge").await;
    let other = create_tenant(db.database(), "enroll-other").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (_app, agent) = routers(db.database()).await;

    // A forged `tenant_id` is an unknown field, so the request is refused.
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let mut forged = body(&token, &p256_csr());
    forged["tenant_id"] = json!(other.to_string());
    let (status, _, _) = send(
        &agent,
        request("POST", "/agent/v1/enroll", None, Some(forged)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The same token still enrols, because the failed request consumed nothing.
    let (status, response) = enroll(&agent, &token, &csr_with_subject("evil.example")).await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    let agent_id = response
        .get("agent_id")
        .and_then(Value::as_str)
        .expect("the agent id");
    let cert_der = decode_cert(response["certificate"].as_str().expect("cert"));
    let cert = parse(&cert_der);
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .expect("the common name is present")
        .as_str()
        .expect("the common name is text");
    assert_eq!(cn, agent_id, "the forged subject must be ignored");

    db.cleanup().await;
}

/// The request policy of ADR-0026: an ECDSA P-384 request is refused with HTTP 400 on the field
/// `csr`.
#[tokio::test]
async fn a_p384_csr_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-p384").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let (_app, agent) = routers(db.database()).await;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).expect("cannot generate the key");
    let csr = CertificateParams::default()
        .serialize_request(&key)
        .expect("cannot build the CSR")
        .der()
        .to_vec();
    let (status, problem) = enroll(&agent, &token, &csr).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem.get("field").and_then(Value::as_str), Some("csr"));

    db.cleanup().await;
}

/// One token creates one agent: concurrent requests with one token give one success.
#[tokio::test]
async fn ac_id_01_concurrent_requests_with_one_token_create_one_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-race").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let (app, agent) = routers(db.database()).await;

    let mut handles = Vec::new();
    for _ in 0..4 {
        let agent = agent.clone();
        let token = token.clone();
        handles.push(tokio::spawn(async move {
            enroll(&agent, &token, &p256_csr()).await.0
        }));
    }
    let mut successes = 0;
    for handle in handles {
        if handle.await.unwrap_or(StatusCode::INTERNAL_SERVER_ERROR) == StatusCode::CREATED {
            successes += 1;
        }
    }
    assert_eq!(successes, 1, "exactly one request must create the agent");

    let reader = seed_session(db.database(), tenant, "reader", "viewer").await;
    assert_eq!(agents_of(&app, &reader).await.len(), 1);

    db.cleanup().await;
}

/// The request policy reads the curve from the public key, not from the signature algorithm.
///
/// A P-384 key with an `ecdsa-with-SHA256` signature names the P-256 algorithm in the signature
/// OID, so a check that reads the signature algorithm alone accepts it.
#[tokio::test]
async fn a_p384_key_with_a_sha256_signature_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let Some(csr) = p384_sha256_csr() else {
        println!("skipped: openssl is not on PATH, so the test cannot build the request");
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-p384-sha256").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (token, id) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let (_app, agent) = routers(db.database()).await;

    let (status, problem) = enroll(&agent, &token, &csr).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem.get("field").and_then(Value::as_str), Some("csr"));

    // The request check runs before the token lookup, so the token stays unused.
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let entry = enrollment_tokens::get(&mut tx, id)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("the token row is present");
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    assert!(
        entry.used_at.is_none(),
        "a bad request must not spend the token"
    );

    db.cleanup().await;
}

/// A bad certificate signing request does not spend a live token: the same token still enrolls.
#[tokio::test]
async fn a_live_token_and_a_bad_csr_leave_the_token_unused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let Some(rsa) = rsa_csr() else {
        println!("skipped: openssl is not on PATH, so the test cannot build the request");
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-bad-csr").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (token, id) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let (_app, agent) = routers(db.database()).await;

    let (status, problem) = enroll(&agent, &token, &rsa).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem.get("field").and_then(Value::as_str), Some("csr"));

    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let entry = enrollment_tokens::get(&mut tx, id)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("the token row is present");
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    assert!(
        entry.used_at.is_none(),
        "the bad request must not spend the token"
    );

    // The same token still enrols with a valid request.
    let (status, response) = enroll(&agent, &token, &p256_csr()).await;
    assert_eq!(status, StatusCode::CREATED, "{response}");

    db.cleanup().await;
}

/// A missing job signing key set refuses the enrollment with HTTP 500 and spends no token.
#[tokio::test]
async fn a_missing_signing_key_set_refuses_without_spending_the_token() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-no-keys").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (token, id) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let agent = agent_router(ca_without_signing_state(db.database()).await);

    let (status, _) = enroll(&agent, &token, &p256_csr()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let entry = enrollment_tokens::get(&mut tx, id)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("the token row is present");
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    assert!(
        entry.used_at.is_none(),
        "the missing key set must not spend the token"
    );
    assert_eq!(count(db.database(), tenant, "agents").await, 0);

    db.cleanup().await;
}

fn openssl_available() -> bool {
    std::process::Command::new("openssl")
        .arg("version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("restichub-enroll-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("cannot create the temporary directory");
    dir
}

/// Builds a P-384 key request signed with SHA-256, so the signature OID names P-256.
fn p384_sha256_csr() -> Option<Vec<u8>> {
    if !openssl_available() {
        return None;
    }
    let dir = temp_dir();
    let key = dir.join("key.pem");
    let csr = dir.join("csr.der");
    let generated = std::process::Command::new("openssl")
        .args(["ecparam", "-name", "secp384r1", "-genkey", "-noout"])
        .arg("-out")
        .arg(&key)
        .output()
        .ok()?;
    if !generated.status.success() {
        std::fs::remove_dir_all(&dir).ok();
        return None;
    }
    let requested = std::process::Command::new("openssl")
        .args(["req", "-new", "-key"])
        .arg(&key)
        .args(["-sha256", "-subj", "/CN=agent", "-outform", "DER"])
        .arg("-out")
        .arg(&csr)
        .output()
        .ok()?;
    let der = requested
        .status
        .success()
        .then(|| std::fs::read(&csr).expect("cannot read the P-384 request"));
    std::fs::remove_dir_all(&dir).ok();
    der
}

/// Builds an RSA request with openssl.
fn rsa_csr() -> Option<Vec<u8>> {
    if !openssl_available() {
        return None;
    }
    let dir = temp_dir();
    let key = dir.join("key.pem");
    let csr = dir.join("csr.der");
    let requested = std::process::Command::new("openssl")
        .args([
            "req",
            "-new",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=agent",
        ])
        .arg("-keyout")
        .arg(&key)
        .arg("-outform")
        .arg("DER")
        .arg("-out")
        .arg(&csr)
        .output()
        .ok()?;
    let der = requested
        .status
        .success()
        .then(|| std::fs::read(&csr).expect("cannot read the RSA request"));
    std::fs::remove_dir_all(&dir).ok();
    der
}
