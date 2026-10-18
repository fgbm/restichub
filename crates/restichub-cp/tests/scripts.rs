//! Server scripts on the control plane side: the templates, the profile scripts, the trust mode
//! of the agents, the job builder and the log of one step (FR-HK-08 to FR-HK-12, AC-HK-07,
//! AC-HK-09, AC-HK-11, AC-HK-12, ADR-0037).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_proto::agent::v1::{
    AgentMessage, LogChunk, LogStream, ScriptInterpreter, agent_message, script_env, step,
};
use serde_json::{Value, json};

use queue_support::{Env, eventually, test_queue};

const CANARY: &str = "CANARY-script-secret-90b2";
const BODY_CANARY: &str = "CANARY-script-body-text";
const ENV_CANARY: &str = "CANARY-script-env-value";

fn bash() -> Vec<ScriptInterpreter> {
    vec![ScriptInterpreter::Bash, ScriptInterpreter::Sh]
}

fn script(name: &str, phase: &str, env: Value) -> Value {
    json!({
        "name": name,
        "interpreter": "bash",
        "phase": phase,
        "body": format!("echo {name} # {BODY_CANARY}\n"),
        "timeout_seconds": 60,
        "failure_policy": "abort_backup",
        "env": env,
    })
}

async fn create_secret(env: &Env, name: &str, purpose: &str) -> String {
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/secrets",
            Some(json!({"name": name, "purpose": purpose, "value": CANARY})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().expect("the secret id").to_owned()
}

fn template_body(name: &str) -> Value {
    json!({
        "name": name,
        "description": "dump one database",
        "interpreter": "bash",
        "phase": "before",
        "body": format!("pg_dump \"$PGDATABASE\" # {BODY_CANARY}\n"),
        "timeout_seconds": 600,
        "failure_policy": "abort_backup",
        "parameters": [
            {"name": "PGDATABASE", "label": "Database", "type": "string", "default": "app"},
            {"name": "PGPASSWORD", "label": "Password", "type": "secret_ref"},
        ],
    })
}

async fn audit_rows(env: &Env, action: &str) -> Vec<String> {
    let mut tx = env.scoped().await;
    let rows = sqlx::query_scalar(
        "SELECT coalesce(diff::text, '') || ' ' || coalesce(object_id::text, '') \
         FROM audit_log WHERE action = $1 ORDER BY occurred_at",
    )
    .bind(action)
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    rows
}

/// AC-HK-12: an Admin creates, reads, changes and removes a template, and every change writes the
/// hash of the script and never the text.
#[tokio::test]
async fn ac_hk_12_a_template_change_writes_the_hash_in_the_audit_log() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (status, created) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/script-templates",
            Some(template_body("pg dump")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().expect("the id").to_owned();
    assert_eq!(created["builtin"], false);
    assert_eq!(created["parameters"][0]["default"], "app");
    assert_eq!(created["parameters"][1]["type"], "secret_ref");

    let (status, list) = env
        .call(&env.admin, "GET", "/api/v1/script-templates", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let items = list["items"].as_array().expect("the items");
    assert!(
        items
            .iter()
            .any(|item| item["id"] == "builtin:postgres-dump-bash" && item["builtin"] == true)
    );
    assert!(items.iter().any(|item| item["id"] == id.as_str()));

    let (status, updated) = env
        .call(
            &env.admin,
            "PATCH",
            &format!("/api/v1/script-templates/{id}"),
            Some(json!({"body": "echo changed\n", "timeout_seconds": 30})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["body"], "echo changed\n");
    assert_eq!(updated["name"], "pg dump");

    let (status, _) = env
        .call(
            &env.admin,
            "DELETE",
            &format!("/api/v1/script-templates/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/script-templates/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let create = audit_rows(&env, "script_template_create").await;
    let update = audit_rows(&env, "script_template_update").await;
    let delete = audit_rows(&env, "script_template_delete").await;
    assert_eq!((create.len(), update.len(), delete.len()), (1, 1, 1));
    let hash =
        restichub_cp::scripts::body_hash(&format!("pg_dump \"$PGDATABASE\" # {BODY_CANARY}\n"));
    assert!(create[0].contains(&hash), "{create:?}");
    assert!(update[0].contains(&restichub_cp::scripts::body_hash("echo changed\n")));
    for row in create.iter().chain(&update).chain(&delete) {
        assert!(
            !row.contains(BODY_CANARY),
            "the audit holds the text: {row}"
        );
    }
    env.db.cleanup().await;
}

/// AC-HK-12: the roles. An Operator reads the templates and a Viewer does not; only an Admin edits them.
#[tokio::test]
async fn ac_hk_12_only_an_admin_edits_the_templates() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (_, created) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/script-templates",
            Some(template_body("shared")),
        )
        .await;
    let id = created["id"].as_str().expect("the id").to_owned();
    for path in ["/api/v1/script-templates", "/api/v1/job-templates"] {
        let (status, _) = env.call(&env.operator, "GET", path, None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let (status, _) = env.call(&env.viewer, "GET", path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        let (status, _) = env.call("not-a-token", "GET", path, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    for token in [&env.viewer, &env.operator] {
        let (status, _) = env
            .call(
                token,
                "POST",
                "/api/v1/script-templates",
                Some(template_body("other")),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = env
            .call(
                token,
                "PATCH",
                &format!("/api/v1/script-templates/{id}"),
                Some(json!({"body": "echo x\n"})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = env
            .call(
                token,
                "DELETE",
                &format!("/api/v1/script-templates/{id}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = env
            .call(
                token,
                "POST",
                "/api/v1/job-templates",
                Some(json!({"name": "job", "scripts": []})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    // A built-in template is read-only for an Admin too.
    let (status, problem) = env
        .call(
            &env.admin,
            "DELETE",
            "/api/v1/script-templates/builtin:postgres-dump-bash",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    let (status, _) = env
        .call(
            &env.admin,
            "PATCH",
            "/api/v1/job-templates/builtin:postgres-linux",
            Some(json!({"name": "x"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    env.db.cleanup().await;
}

/// AC-TN-01: a template of another tenant gives 404.
#[tokio::test]
async fn ac_tn_01_a_template_of_another_tenant_is_not_found() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (_, created) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/script-templates",
            Some(template_body("mine")),
        )
        .await;
    let id = created["id"].as_str().expect("the id").to_owned();
    let (_, job) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/job-templates",
            Some(json!({"name": "mine", "scripts": []})),
        )
        .await;
    let job_id = job["id"].as_str().expect("the id").to_owned();
    let other = common::create_tenant_with_key(env.db.database(), "other-tenant").await;
    let token = common::seed_api_token(env.db.database(), other, "admin").await;
    for path in [
        format!("/api/v1/script-templates/{id}"),
        format!("/api/v1/job-templates/{job_id}"),
    ] {
        let (status, _) = env.call(&token, "GET", &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        let (status, _) = env.call(&token, "DELETE", &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    let (status, list) = env
        .call(&token, "GET", "/api/v1/script-templates", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        list["items"]
            .as_array()
            .expect("the items")
            .iter()
            .all(|item| item["builtin"] == true)
    );
    env.db.cleanup().await;
}

/// FR-HK-12: a job template keeps an ordered set of scripts, with an empty secret reference.
#[tokio::test]
async fn fr_hk_12_a_job_template_keeps_the_ordered_scripts() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let scripts = json!([
        script("first", "before", json!([])),
        script(
            "second",
            "before",
            json!([{"name": "PGPASSWORD", "kind": "secret_ref", "value": ""}])
        ),
        script("cleanup", "finally", json!([])),
    ]);
    let (status, created) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/job-templates",
            Some(json!({"name": "database", "description": "d", "scripts": scripts})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let names: Vec<&str> = created["scripts"]
        .as_array()
        .expect("the scripts")
        .iter()
        .filter_map(|script| script["name"].as_str())
        .collect();
    assert_eq!(names, ["first", "second", "cleanup"]);
    assert_eq!(created["scripts"][1]["env"][0]["value"], "");
    let rows = audit_rows(&env, "job_template_create").await;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].contains(BODY_CANARY));
    assert!(rows[0].contains("sha256"));
    env.db.cleanup().await;
}

/// AC-HK-12: only an Admin changes the scripts of a profile; the audit holds the hash.
#[tokio::test]
async fn ac_hk_12_only_an_admin_changes_the_scripts_of_a_profile_and_a_viewer_reads_no_body() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let body = |scripts: Value, name: &str| {
        json!({
            "name": name,
            "paths": {"all": ["/srv"]},
            "scripts": scripts,
        })
    };
    // An Operator cannot create a profile with a script.
    let (status, _) = env
        .call(
            &env.operator,
            "POST",
            "/api/v1/profiles",
            Some(body(
                json!([script("dump", "before", json!([]))]),
                "by-operator",
            )),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // An Operator can create a profile without one.
    let (status, plain) = env
        .call(
            &env.operator,
            "POST",
            "/api/v1/profiles",
            Some(body(json!([]), "plain")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    let plain_id = plain["profile"]["id"].as_str().expect("the id").to_owned();
    let (status, _) = env
        .call(
            &env.operator,
            "PATCH",
            &format!("/api/v1/profiles/{plain_id}"),
            Some(body(json!([script("dump", "before", json!([]))]), "plain")),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let inline = json!([{"name": "PGPASSWORD", "kind": "string", "value": ENV_CANARY}]);
    let (status, created) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/profiles",
            Some(body(
                json!([script("dump", "before", inline.clone())]),
                "with-script",
            )),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["profile"]["scripts"][0]["name"], "dump");
    let id = created["profile"]["id"]
        .as_str()
        .expect("the id")
        .to_owned();
    // A Viewer reads the profile without the script bodies; an Operator reads them.
    for uri in [
        "/api/v1/profiles".to_owned(),
        format!("/api/v1/profiles/{id}"),
    ] {
        let (status, as_viewer) = env.call(&env.viewer, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{as_viewer}");
        assert!(
            !as_viewer.to_string().contains(BODY_CANARY),
            "a Viewer reads a script body: {as_viewer}"
        );
        assert!(
            !as_viewer.to_string().contains(ENV_CANARY),
            "a Viewer reads a script variable: {as_viewer}"
        );
        assert!(as_viewer.to_string().contains("dump"), "{as_viewer}");
        let (_, as_operator) = env.call(&env.operator, "GET", &uri, None).await;
        assert!(
            as_operator.to_string().contains(BODY_CANARY),
            "{as_operator}"
        );
        assert!(
            as_operator.to_string().contains(ENV_CANARY),
            "{as_operator}"
        );
    }
    // An Operator can change another setting while the scripts stay as they are.
    let unchanged = body(json!([script("dump", "before", inline)]), "with-script");
    let (status, saved) = env
        .call(
            &env.operator,
            "PATCH",
            &format!("/api/v1/profiles/{id}"),
            Some(unchanged),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, _) = env
        .call(
            &env.operator,
            "PATCH",
            &format!("/api/v1/profiles/{id}"),
            Some(body(
                json!([script("dump2", "before", json!([]))]),
                "with-script",
            )),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The audit record names the script and holds its hash, not its text.
    let rows = audit_rows(&env, "profile_create").await;
    let hash = restichub_cp::scripts::body_hash(&format!("echo dump # {BODY_CANARY}\n"));
    assert!(rows.iter().any(|row| row.contains(&hash)), "{rows:?}");
    for row in audit_rows(&env, "profile_create")
        .await
        .iter()
        .chain(&audit_rows(&env, "profile_update").await)
    {
        assert!(!row.contains(BODY_CANARY), "{row}");
    }
    env.db.cleanup().await;
}

/// ADR-0017: a script variable that names a secret of another purpose is refused.
#[tokio::test]
async fn adr_0017_a_script_variable_takes_a_hook_secret_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let other = create_secret(&env, "repo.key", "repository_password").await;
    let missing = uuid::Uuid::new_v4().to_string();
    for value in [other, missing] {
        let (status, problem) = env
            .call(
                &env.admin,
                "POST",
                "/api/v1/profiles",
                Some(json!({
                    "name": "bad",
                    "paths": {"all": ["/srv"]},
                    "scripts": [script("dump", "before", json!([
                        {"name": "PGPASSWORD", "kind": "secret_ref", "value": value}
                    ]))],
                })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
        assert_eq!(problem["field"], "scripts[0].env[0].value");
    }
    env.db.cleanup().await;
}

/// FR-HK-09: a script that breaks a rule names its field.
#[tokio::test]
async fn fr_hk_09_a_broken_script_is_refused_with_its_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let cases = [
        (json!({"body": "  "}), "scripts[0].body"),
        (json!({"body": "x".repeat(65 * 1024)}), "scripts[0].body"),
        (
            json!({"env": [{"name": "LD_PRELOAD", "kind": "string", "value": "x"}]}),
            "scripts[0].env[0].name",
        ),
        (
            json!({"env": [{"name": "A", "kind": "string", "value": "1"}, {"name": "a", "kind": "string", "value": "2"}]}),
            "scripts[0].env[1].name",
        ),
        (json!({"name": ""}), "scripts[0].name"),
        (
            json!({"timeout_seconds": 100_000}),
            "scripts[0].timeout_seconds",
        ),
    ];
    for (patch, field) in cases {
        let mut broken = script("dump", "before", json!([]));
        for (key, value) in patch.as_object().expect("an object") {
            broken[key] = value.clone();
        }
        let (status, problem) = env
            .call(
                &env.admin,
                "POST",
                "/api/v1/profiles",
                Some(json!({
                    "name": "bad",
                    "paths": {"all": ["/srv"]},
                    "scripts": [broken],
                })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {problem}");
        assert_eq!(problem["field"], field, "{problem}");
    }
    env.db.cleanup().await;
}

/// FR-HK-08: the agent view shows the trust mode and the interpreters.
#[tokio::test]
async fn fr_hk_08_the_agent_view_shows_the_trust_mode() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let on = env.connect_agent_trusting("host-on", true, bash()).await;
    let off = env.connect_agent_trusting("host-off", false, bash()).await;
    let old = env.connect_agent("host-old").await;
    let view = |agent: &queue_support::FakeAgent| {
        let uri = format!("/api/v1/agents/{}", agent.id);
        let env = &env;
        async move { env.call(&env.viewer, "GET", &uri, None).await.1 }
    };
    let on = view(&on).await;
    assert_eq!(on["server_scripts"], "enabled");
    assert_eq!(on["script_interpreters"], json!(["bash", "sh"]));
    let off = view(&off).await;
    assert_eq!(off["server_scripts"], "disabled");
    assert_eq!(off["script_interpreters"], json!([]));
    assert_eq!(view(&old).await["server_scripts"], "unknown");
    env.db.cleanup().await;
}

/// AC-HK-07: the control plane refuses a plan for a host without the trust mode, with a typed
/// reason that names the field.
#[tokio::test]
async fn ac_hk_07_a_plan_for_a_host_without_the_trust_mode_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let trusting = env.connect_agent_trusting("host-on", true, bash()).await;
    let closed = env.connect_agent_trusting("host-off", false, bash()).await;
    // A remote backend, so the plan of either host passes the repository host rule.
    let repository = env.create_rest_repository(trusting.id, "user", "pw").await;
    let profile = env
        .create_profile(json!({"scripts": [script("dump", "before", json!([]))]}))
        .await;
    let plan = |agent: &queue_support::FakeAgent, name: &str| {
        json!({
            "name": name,
            "profile_id": profile.to_string(),
            "agent_id": agent.id.to_string(),
            "repository_id": repository.to_string(),
            "cron": "0 2 1 1 *",
            "time_zone": "UTC",
            "missed_run_policy": "skip",
        })
    };
    let (status, problem) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/plans",
            Some(plan(&closed, "closed")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "scripts[0]");
    assert_eq!(problem["reason"], "server_scripts_disabled");
    let (status, created) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/plans",
            Some(plan(&trusting, "open")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    env.db.cleanup().await;
}

/// AC-HK-07: the builder fails a job when the host turned the mode off after the plan was saved.
#[tokio::test]
async fn ac_hk_07_the_builder_refuses_a_job_for_a_host_that_turned_the_mode_off() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent_trusting("host-a", true, bash()).await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({"scripts": [script("dump", "before", json!([]))]}))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    // The host administrator turned the mode off and the agent restarted.
    let capabilities = restichub_proto::AgentCapability::ALL
        .iter()
        .map(|capability| capability.as_str().to_owned())
        .collect();
    let mut agent = env
        .reconnect_agent_reporting(
            &agent,
            capabilities,
            queue_support::scripts_report(false, bash()),
        )
        .await;
    // The control plane refuses a manual run at once, with the typed reason.
    let (status, problem) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/plans/{plan}/run"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["reason"], "server_scripts_disabled");
    // A job that was queued before the change still fails in the builder with the same reason.
    let job = env
        .enqueue(
            restichub_db::JobKind::Backup,
            agent.id,
            repository,
            Some(plan),
        )
        .await;
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["result_code"], "config_error");
    assert_eq!(body["result_message"], "server_scripts_disabled");
    assert!(
        agent
            .try_job(std::time::Duration::from_millis(300))
            .await
            .is_none()
    );
    env.db.cleanup().await;
}

/// FR-PR-09: an agent without the capability `scripts-v1` never gets a script step.
#[tokio::test]
async fn fr_pr_09_an_agent_without_scripts_v1_never_gets_a_script_step() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent_trusting("host-a", true, bash()).await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({"scripts": [script("dump", "before", json!([]))]}))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let mut agent = env
        .reconnect_agent(&agent, vec!["restore-v1".to_owned()])
        .await;
    let job = env
        .enqueue(
            restichub_db::JobKind::Backup,
            agent.id,
            repository,
            Some(plan),
        )
        .await;
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["result_message"], "server_scripts_disabled");
    assert!(
        agent
            .try_job(std::time::Duration::from_millis(300))
            .await
            .is_none()
    );
    env.db.cleanup().await;
}

/// FR-HK-09, ADR-0017: the builder resolves the secret at dispatch time, records the delivery and
/// the hash of the script, and lists every step before it runs.
#[tokio::test]
async fn ac_hk_11_the_signed_job_carries_the_resolved_secret_and_the_planned_steps() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent_trusting("host-a", true, bash()).await;
    let repository = env
        .create_repository(agent.id, "pw-of-the-repository")
        .await;
    let secret = create_secret(&env, "db.password", "hook_secret").await;
    let scripts = json!([
        script(
            "dump",
            "before",
            json!([
                {"name": "PGHOST", "kind": "string", "value": "db1"},
                {"name": "PGPASSWORD", "kind": "secret_ref", "value": secret},
            ])
        ),
        script("cleanup", "finally", json!([])),
        script("check", "before", json!([])),
    ]);
    let profile = env.create_profile(json!({"scripts": scripts})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job_id = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;

    let names: Vec<(String, &str)> = job
        .steps
        .iter()
        .map(|planned| match planned.kind.as_ref() {
            Some(step::Kind::Script(script)) => (script.name.clone(), "script"),
            Some(step::Kind::Backup(_)) => ("backup".to_owned(), "backup"),
            _ => ("other".to_owned(), "other"),
        })
        .collect();
    assert_eq!(
        names,
        [
            ("dump".to_owned(), "script"),
            ("check".to_owned(), "script"),
            ("backup".to_owned(), "backup"),
            ("cleanup".to_owned(), "script"),
        ]
    );
    let Some(step::Kind::Script(dump)) = &job.steps[0].kind else {
        panic!("the first step is a script");
    };
    let password = dump
        .env
        .iter()
        .find(|variable| variable.name == "PGPASSWORD")
        .expect("the password variable");
    assert_eq!(
        password.value,
        Some(script_env::Value::SecretValue(CANARY.to_owned()))
    );
    assert_eq!(job.steps[0].step_id, "script-0");

    let detail = env.job(job_id).await;
    let planned: Vec<(&str, &str, &str, &str)> = detail["planned"]
        .as_array()
        .expect("the planned steps")
        .iter()
        .map(|entry| {
            (
                entry["step_id"].as_str().unwrap_or_default(),
                entry["kind"].as_str().unwrap_or_default(),
                entry["name"].as_str().unwrap_or_default(),
                entry["phase"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        planned,
        [
            ("script-0", "script", "dump", "before"),
            ("script-2", "script", "check", "before"),
            ("backup", "backup", "", ""),
            ("script-1", "script", "cleanup", "finally"),
        ]
    );
    assert_eq!(
        detail["planned_steps"],
        json!(["script-0", "script-2", "backup", "script-1"])
    );
    // The job record holds the hash of each script body and none for another step (ADR-0037).
    let hashes: Vec<Option<&str>> = detail["planned"]
        .as_array()
        .expect("the planned steps")
        .iter()
        .map(|entry| entry["content_sha256"].as_str())
        .collect();
    let dump_hash = restichub_cp::scripts::body_hash(&format!("echo dump # {BODY_CANARY}\n"));
    assert_eq!(hashes[0], Some(dump_hash.as_str()));
    assert_eq!(hashes[2], None);

    // The delivery names the secret and never its value (AC-RP-03).
    let delivered = audit_rows(&env, "secret_delivered").await;
    assert!(
        delivered.iter().any(|row| row.contains(&secret)),
        "{delivered:?}"
    );
    // The run record has one hash for each script and no text, no value.
    let runs = audit_rows(&env, "script_run").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    let hash = restichub_cp::scripts::body_hash(&format!("echo dump # {BODY_CANARY}\n"));
    assert!(runs[0].contains(&hash), "{}", runs[0]);
    assert!(runs[0].contains("cleanup"));
    for row in delivered.iter().chain(&runs) {
        assert!(!row.contains(CANARY) && !row.contains(BODY_CANARY), "{row}");
    }
    env.db.cleanup().await;
}

/// AC-HK-11: the log API returns the lines of one step, and the detail names every step.
#[tokio::test]
async fn ac_hk_11_the_log_api_returns_the_lines_of_one_step() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent_trusting("host-a", true, bash()).await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({"scripts": [
            script("dump", "before", json!([])),
            script("cleanup", "finally", json!([])),
        ]}))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let id = queue_support::decode_job_id(&signed);
    agent.start(&env, &id).await;
    for (sequence, (step_id, text)) in [
        ("script-0", "dumping\n"),
        ("backup", "scanning\n"),
        ("script-1", "cleaning\n"),
        ("script-0", "dumped\n"),
    ]
    .into_iter()
    .enumerate()
    {
        agent
            .tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::LogChunk(LogChunk {
                    job_id: id.clone(),
                    step_id: step_id.to_owned(),
                    stream: LogStream::Stdout as i32,
                    sequence: sequence as u64 + 1,
                    data: text.as_bytes().to_vec(),
                })),
            })
            .await
            .expect("cannot send the log");
    }
    eventually(|| async { env.job(job).await["log"]["lines"] == 4 }).await;

    let lines = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .expect("the items")
            .iter()
            .map(|line| line["text"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    let all = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs/{job}/logs"),
            None,
        )
        .await
        .1;
    assert_eq!(lines(&all).len(), 4);
    let dump = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs/{job}/logs?step_id=script-0"),
            None,
        )
        .await
        .1;
    assert_eq!(lines(&dump), ["dumping", "dumped"]);
    let backup = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs/{job}/logs?step_id=backup"),
            None,
        )
        .await
        .1;
    assert_eq!(lines(&backup), ["scanning"]);
    let none = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs/{job}/logs?step_id=script-9"),
            None,
        )
        .await
        .1;
    assert_eq!(lines(&none).len(), 0);
    let second = env
        .call(
            &env.viewer,
            "GET",
            &format!(
                "/api/v1/jobs/{job}/logs?step_id=script-0&after_seq={}",
                dump["items"][0]["seq"]
            ),
            None,
        )
        .await
        .1;
    assert_eq!(lines(&second), ["dumped"]);
    env.db.cleanup().await;
}

/// AC-PF-09: a secret parameter reaches the agent as a secret value, and a text parameter as a value
/// that the agent masks (FR-PF-10, FR-PF-12).
#[tokio::test]
async fn ac_pf_09_a_parameter_of_a_script_variable_reaches_the_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env
        .connect_agent_trusting("host-params", true, bash())
        .await;
    let repository = env
        .create_repository(agent.id, "pw-of-the-repository")
        .await;
    let secret = create_secret(&env, "db.password", "hook_secret").await;
    let scripts = json!([script(
        "dump",
        "before",
        json!([
            {"name": "PGDIR", "kind": "parameter", "value": "pg_dir"},
            {"name": "PGPASSWORD", "kind": "parameter", "value": "pg_password"},
        ])
    )]);
    let profile = env
        .create_profile(json!({
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": [
                {"name": "pg_dir", "kind": "text", "required": true},
                {"name": "pg_password", "kind": "secret", "required": true},
            ],
            "scripts": scripts,
        }))
        .await;
    let (status, body) = env
        .call(
            &env.admin,
            "PUT",
            &format!("/api/v1/agents/{}/parameters", agent.id),
            Some(json!({"parameters": [
                {"name": "pg_dir", "kind": "text", "value": "/srv/pg"},
                {"name": "pg_password", "kind": "secret", "secret_id": secret},
            ]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plan = env.create_plan(profile, agent.id, repository).await;
    env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;

    let Some(step::Kind::Script(dump)) = &job.steps[0].kind else {
        panic!("the first step is a script");
    };
    let value = |name: &str| {
        dump.env
            .iter()
            .find(|variable| variable.name == name)
            .and_then(|variable| variable.value.clone())
    };
    // The secret value comes from the hook secret of the tenant, and the audit records it.
    assert_eq!(
        value("PGPASSWORD"),
        Some(script_env::Value::SecretValue(CANARY.to_owned()))
    );
    // A text value keeps the mask marker, so the agent hides it in the step log.
    assert_eq!(
        value("PGDIR"),
        Some(script_env::Value::MaskedValue("/srv/pg".to_owned()))
    );
    let Some(step::Kind::Backup(backup)) = &job.steps[1].kind else {
        panic!("the second step is the backup");
    };
    assert_eq!(backup.paths, vec!["/data//srv/pg".to_owned()]);
    assert_eq!(
        audit_rows(&env, "secret_delivered").await.len(),
        2,
        "the password of the repository and the parameter secret"
    );
}
