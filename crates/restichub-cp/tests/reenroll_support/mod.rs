//! Shared helpers of the re-registration tests (FR-ID-10, FR-ID-11).

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use axum::Router;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use restichub_db::{AgentId, TenantId};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

use crate::common::{
    agent_router, api_router, create_tenant_with_key, create_user, pki_state, request,
    seed_enrollment_token_value, send, session_cookie,
};

pub const MACHINE: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";

pub struct Env {
    pub api: Router,
    pub agent: Router,
    pub cookie: String,
    pub tenant: TenantId,
    pub creator: restichub_db::UserId,
}

pub async fn env(db: &restichub_db::test_util::TestDatabase, name: &str) -> Env {
    let tenant = create_tenant_with_key(db.database(), name).await;
    let creator = create_user(
        db.database(),
        tenant,
        &format!("admin-{name}"),
        "phc",
        "admin",
    )
    .await;
    let cookie = session_cookie(db.database(), tenant, creator).await;
    let (state, _ca) = pki_state(db.database()).await;
    Env {
        api: api_router(state.clone()),
        agent: agent_router(state),
        cookie,
        tenant,
        creator,
    }
}

pub fn csr() -> Vec<u8> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    CertificateParams::default()
        .serialize_request(&key)
        .expect("cannot build the CSR")
        .der()
        .to_vec()
}

pub async fn enroll(
    env: &Env,
    token: &str,
    hostname: &str,
    machine_id: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({
        "token": token,
        "csr": STANDARD.encode(csr()),
        "hostname": hostname,
        "os": "windows",
        "arch": "amd64",
        "agent_version": "0.1.0+dev",
        "restic_version": "0.19.0",
    });
    if let Some(machine_id) = machine_id {
        body["machine_id"] = json!(machine_id);
    }
    let (status, _, response) = send(
        &env.agent,
        request("POST", "/agent/v1/enroll", None, Some(body)),
    )
    .await;
    (status, response)
}

pub async fn fresh_token(db: &restichub_db::test_util::TestDatabase, env: &Env) -> String {
    seed_enrollment_token_value(
        db.database(),
        env.tenant,
        env.creator,
        &["host".to_owned()],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await
    .0
}

pub async fn enroll_new(
    db: &restichub_db::test_util::TestDatabase,
    env: &Env,
    hostname: &str,
    machine_id: Option<&str>,
) -> AgentId {
    let token = fresh_token(db, env).await;
    let (status, response) = enroll(env, &token, hostname, machine_id).await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    assert_eq!(response["reenrolled"], false);
    agent_id(&response)
}

pub fn agent_id(response: &Value) -> AgentId {
    response["agent_id"]
        .as_str()
        .and_then(|text| text.parse().ok())
        .expect("the response holds the agent id")
}

pub async fn agents(env: &Env) -> Vec<Value> {
    let (status, _, page) = send(
        &env.api,
        request("GET", "/api/v1/agents", Some(&env.cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    page["items"].as_array().cloned().expect("a list")
}

pub fn find(items: &[Value], id: AgentId) -> &Value {
    items
        .iter()
        .find(|item| item["id"] == id.to_string())
        .expect("the agent is listed")
}

pub async fn audit_count(env: &Env, action: &str) -> usize {
    let (status, _, page) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/audit?action={action}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    page["items"].as_array().expect("a list").len()
}

pub async fn issue_reenroll_token(env: &Env, agent: AgentId) -> (StatusCode, Value) {
    let (status, _, body) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{agent}/reenroll"),
            Some(&env.cookie),
            Some(json!({})),
        ),
    )
    .await;
    (status, body)
}

pub async fn serial_of(
    db: &restichub_db::test_util::TestDatabase,
    env: &Env,
    id: AgentId,
) -> (Vec<u8>, String) {
    let mut tx = db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let agent = restichub_db::agents::get(&mut tx, id)
        .await
        .expect("cannot read")
        .expect("the agent exists");
    let hex = agent
        .certificate_serial
        .iter()
        .skip_while(|byte| **byte == 0)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    (agent.certificate_serial, hex)
}
