//! Re-registration, duplicate detection and the waiting state of a host (FR-ID-01, FR-ID-04,
//! FR-TN-01).

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use restichub_db::{AgentId, TenantId};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

use common::{
    agent_router, api_router, create_tenant_with_key, create_user, pki_state, request,
    seed_enrollment_token_value, seed_plan, seed_profile, seed_repository, seed_secret, send,
    session_cookie,
};

const MACHINE: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";

struct Env {
    api: Router,
    agent: Router,
    cookie: String,
    tenant: TenantId,
    creator: restichub_db::UserId,
}

async fn env(db: &restichub_db::test_util::TestDatabase, name: &str) -> Env {
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

fn csr() -> Vec<u8> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    CertificateParams::default()
        .serialize_request(&key)
        .expect("cannot build the CSR")
        .der()
        .to_vec()
}

async fn enroll(
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

async fn fresh_token(db: &restichub_db::test_util::TestDatabase, env: &Env) -> String {
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

async fn enroll_new(
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

fn agent_id(response: &Value) -> AgentId {
    response["agent_id"]
        .as_str()
        .and_then(|text| text.parse().ok())
        .expect("the response holds the agent id")
}

async fn agents(env: &Env) -> Vec<Value> {
    let (status, _, page) = send(
        &env.api,
        request("GET", "/api/v1/agents", Some(&env.cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    page["items"].as_array().cloned().expect("a list")
}

fn find<'a>(items: &'a [Value], id: AgentId) -> &'a Value {
    items
        .iter()
        .find(|item| item["id"] == id.to_string())
        .expect("the agent is listed")
}

async fn audit_count(env: &Env, action: &str) -> usize {
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

async fn issue_reenroll_token(env: &Env, agent: AgentId) -> (StatusCode, Value) {
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

#[tokio::test]
async fn ac_id_01_a_host_waits_for_its_first_connection() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "wait").await;
    let id = enroll_new(&db, &env, "waiting-host", None).await;

    let items = agents(&env).await;
    assert!(find(&items, id)["first_connected_at"].is_null());

    let (_, hash_serial) = serial_of(&db, &env, id).await;
    let mut tx = db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let touched = restichub_db::agents::touch_last_seen_guarded(
        &mut tx,
        id,
        &hash_serial,
        OffsetDateTime::now_utc(),
    )
    .await
    .expect("cannot touch");
    tx.commit().await.expect("cannot commit");
    assert!(touched);

    let items = agents(&env).await;
    assert!(find(&items, id)["first_connected_at"].is_string());
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_01_a_used_token_names_the_host_it_enrolled() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "named").await;
    let id = enroll_new(&db, &env, "named-host", None).await;

    let (status, _, page) = send(
        &env.api,
        request("GET", "/api/v1/enrollment-tokens", Some(&env.cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tokens = page["items"].as_array().expect("a list");
    let used: Vec<_> = tokens
        .iter()
        .filter(|token| token["used_at"].is_string())
        .collect();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0]["used_by_agent_id"], id.to_string());
    db.cleanup().await;
}

async fn serial_of(
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

#[tokio::test]
async fn ac_id_01_a_bound_token_keeps_the_agent_and_replaces_the_identity() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "reenroll").await;
    let id = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let secret = seed_secret(db.database(), env.tenant, "plan.password").await;
    let repository = seed_repository(db.database(), env.tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), env.tenant, "profile", repository).await;
    let plan = seed_plan(db.database(), env.tenant, "plan", profile, id).await;
    let (old_serial, old_hex) = serial_of(&db, &env, id).await;

    let (status, issued) = issue_reenroll_token(&env, id).await;
    assert_eq!(status, StatusCode::CREATED, "{issued}");
    assert_eq!(issued["replaces_agent_id"], id.to_string());
    let token = issued["token"].as_str().expect("the token");

    let (status, response) = enroll(&env, token, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    assert_eq!(response["reenrolled"], true);
    assert_eq!(agent_id(&response), id);

    let items = agents(&env).await;
    assert_eq!(items.len(), 1, "no second record appears");
    let row = find(&items, id);
    assert_eq!(row["status"], "active");
    assert!(row["first_connected_at"].is_null());
    assert!(row["possible_duplicate_of"].is_null());
    assert_eq!(audit_count(&env, "agent_reenroll").await, 1);

    let (new_serial, new_hex) = serial_of(&db, &env, id).await;
    assert_ne!(old_serial, new_serial);
    let mut tx = db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let now = OffsetDateTime::now_utc();
    let old = restichub_db::agents::touch_last_seen_guarded(&mut tx, id, &old_hex, now)
        .await
        .expect("cannot touch");
    let new = restichub_db::agents::touch_last_seen_guarded(&mut tx, id, &new_hex, now)
        .await
        .expect("cannot touch");
    tx.commit().await.expect("cannot commit");
    assert!(!old, "the old certificate is refused at once");
    assert!(new);

    let (status, _, plan_view) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/plans/{plan}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plan_view["agent_id"], id.to_string());

    // The token is single use.
    let (status, _) = enroll(&env, token, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_01_a_bound_token_of_a_revoked_host_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "reenroll-revoked").await;
    let id = enroll_new(&db, &env, "gone", None).await;
    let (status, issued) = issue_reenroll_token(&env, id).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = issued["token"].as_str().expect("the token").to_owned();

    let (status, _, _) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{id}/revoke"),
            Some(&env.cookie),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = enroll(&env, &token, "gone", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = issue_reenroll_token(&env, id).await;
    assert_eq!(status, StatusCode::CONFLICT);
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_01_an_unbound_token_of_the_same_machine_flags_both_records() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "duplicate").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let other = enroll_new(&db, &env, "OTHER", Some("ffffffffffffffffffffffffffffffff")).await;
    let bare = enroll_new(&db, &env, "BARE", None).await;
    assert_ne!(old, new);

    let items = agents(&env).await;
    assert_eq!(
        find(&items, new)["possible_duplicate_of"]["id"],
        old.to_string()
    );
    assert_eq!(
        find(&items, new)["possible_duplicate_of"]["hostname"],
        "MINDCORE"
    );
    assert_eq!(
        find(&items, old)["possible_duplicate_of"]["id"],
        new.to_string()
    );
    assert!(find(&items, other)["possible_duplicate_of"].is_null());
    assert!(find(&items, bare)["possible_duplicate_of"].is_null());
    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_the_same_machine_in_two_tenants_never_matches() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let first = env(&db, "tenant-one").await;
    let second = env(&db, "tenant-two").await;
    let in_first = enroll_new(&db, &first, "MINDCORE", Some(MACHINE)).await;
    let in_second = enroll_new(&db, &second, "MINDCORE", Some(MACHINE)).await;

    assert!(find(&agents(&first).await, in_first)["possible_duplicate_of"].is_null());
    assert!(find(&agents(&second).await, in_second)["possible_duplicate_of"].is_null());

    // A replace across the tenants finds no pair.
    let (status, _, _) = send(
        &second.api,
        request(
            "POST",
            &format!("/api/v1/agents/{in_second}/replace"),
            Some(&second.cookie),
            Some(json!({"old_agent_id": in_first.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        find(&agents(&first).await, in_first)["status"],
        "active",
        "the other tenant is untouched"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_01_replace_moves_plans_and_maintenance_references_and_revokes_the_old_record() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "replace").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let unrelated = enroll_new(&db, &env, "OTHER", Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")).await;

    let secret = seed_secret(db.database(), env.tenant, "plan.password").await;
    let repository = seed_repository(db.database(), env.tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), env.tenant, "profile", repository).await;
    let plan = seed_plan(db.database(), env.tenant, "plan", profile, old).await;
    let maintenance_repository = {
        use restichub_db::repositories::{NewRepository, RepositorySecrets, RepositorySettings};
        let id = restichub_db::RepositoryId::new();
        let mut tx = db
            .database()
            .begin_tenant(env.tenant)
            .await
            .expect("cannot begin");
        restichub_db::repositories::create(
            &mut tx,
            &NewRepository {
                id,
                name: "maintained",
                backend: restichub_db::BackendKind::Local,
                settings: RepositorySettings {
                    local_path: Some("/mnt/backup/maintained"),
                    ..RepositorySettings::default()
                },
                secrets: RepositorySecrets {
                    password: secret,
                    s3_access_key: None,
                    s3_secret_key: None,
                    rest_password: None,
                },
                maintenance_agent_id: Some(old),
                created_by: None,
            },
        )
        .await
        .expect("cannot create the repository");
        tx.commit().await.expect("cannot commit");
        id
    };

    // A pair without a shared machine identifier is refused.
    let (status, _, _) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&env.cookie),
            Some(json!({"old_agent_id": unrelated.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, moved) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&env.cookie),
            Some(json!({"old_agent_id": old.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["plans_moved"], 1);
    assert_eq!(moved["repositories_moved"], 1);

    let items = agents(&env).await;
    assert_eq!(find(&items, old)["status"], "revoked");
    assert_eq!(find(&items, new)["status"], "active");
    assert!(find(&items, new)["possible_duplicate_of"].is_null());
    assert_eq!(audit_count(&env, "agent_replace").await, 1);

    let (_, _, plan_view) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/plans/{plan}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(plan_view["agent_id"], new.to_string());
    let (_, _, repository_view) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/repositories/{maintenance_repository}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(
        repository_view["maintenance_agent_id"],
        new.to_string(),
        "{repository_view}"
    );
    db.cleanup().await;
}
