//! The hooks on the control plane side: the report of the agents, the catalogue, the check of a
//! profile against its target agents and the job builder (FR-HK-03, FR-HK-04, AC-HK-01, AC-HK-05,
//! ADR-0017).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::collections::BTreeMap;

use axum::http::StatusCode;
use restichub_compiler::{AllowListedHook, HookParameterSpec, HookPhase, ParameterKind};
use restichub_proto::agent::v1::{HookAllowListState, hook_parameter::Typed, step};
use serde_json::{Value, json};

use queue_support::{Env, HookReport, test_queue};

const CANARY: &str = "CANARY-hook-secret-value-1f3a";

fn spec(kind: ParameterKind, required: bool) -> HookParameterSpec {
    HookParameterSpec { kind, required }
}

/// The schema of the `pg-dump` hook in these tests.
fn pg_dump() -> AllowListedHook {
    AllowListedHook {
        parameters: BTreeMap::from([
            (
                "database".to_owned(),
                spec(
                    ParameterKind::String {
                        max_length: 63,
                        pattern: Some("[a-z_][a-z0-9_]*".to_owned()),
                    },
                    true,
                ),
            ),
            (
                "jobs".to_owned(),
                spec(
                    ParameterKind::Integer {
                        minimum: Some(1),
                        maximum: Some(8),
                    },
                    false,
                ),
            ),
            (
                "mode".to_owned(),
                spec(
                    ParameterKind::Enum {
                        values: vec!["full".to_owned(), "fast".to_owned()],
                    },
                    false,
                ),
            ),
            ("token".to_owned(), spec(ParameterKind::SecretRef, false)),
        ]),
        phases: vec![HookPhase::Before, HookPhase::Finally],
    }
}

fn report(hooks: &[(&str, AllowListedHook)]) -> HookReport {
    HookReport {
        state: HookAllowListState::Loaded,
        hooks: hooks.iter().map(|(id, hook)| hook.to_proto(id)).collect(),
    }
}

async fn create_secret(env: &Env, name: &str, purpose: &str, value: &str) -> String {
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/secrets",
            Some(json!({"name": name, "purpose": purpose, "value": value})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().expect("the secret id").to_owned()
}

fn profile_body(repository: &uuid::Uuid, name: &str, hooks: Value) -> Value {
    json!({
        "name": name,
        "repository_id": repository.to_string(),
        "paths": {"all": ["/srv"]},
        "hooks": hooks,
    })
}

fn before(parameters: Value) -> Value {
    json!([{"hook_id": "pg-dump", "phase": "before", "parameters": parameters}])
}

/// FR-HK-04: the hello stores the hooks, and the catalogue and the agent view show the schema and
/// no path.
#[tokio::test]
async fn fr_hk_04_the_report_of_the_agents_reaches_the_catalogue() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let first = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let second = env
        .connect_agent_reporting("host-b", report(&[("pg-dump", pg_dump())]))
        .await;
    let (status, catalogue) = env.call(&env.viewer, "GET", "/api/v1/hooks", None).await;
    assert_eq!(status, StatusCode::OK, "{catalogue}");
    assert_eq!(catalogue["agents"].as_array().map(Vec::len), Some(2));
    let hooks = catalogue["hooks"].as_array().expect("the hooks");
    assert_eq!(hooks.len(), 1, "one schema, one entry: {catalogue}");
    assert_eq!(hooks[0]["hook_id"], "pg-dump");
    assert_eq!(hooks[0]["agent_ids"].as_array().map(Vec::len), Some(2));
    assert_eq!(hooks[0]["phases"], json!(["before", "finally"]));
    let parameters = hooks[0]["parameters"].as_array().expect("the parameters");
    let database = parameters
        .iter()
        .find(|parameter| parameter["name"] == "database")
        .expect("the database parameter");
    assert_eq!(database["type"], "string");
    assert_eq!(database["required"], true);
    assert_eq!(database["max_length"], 63);
    assert_eq!(database["pattern"], "[a-z_][a-z0-9_]*");
    let token = parameters
        .iter()
        .find(|parameter| parameter["name"] == "token")
        .expect("the token parameter");
    assert_eq!(token["type"], "secret_ref");
    assert!(
        !catalogue.to_string().contains("/usr"),
        "no path: {catalogue}"
    );

    let (status, view) = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/agents/{}", first.id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["hook_allow_list"], "loaded");
    assert_eq!(view["hooks"][0]["hook_id"], "pg-dump");
    drop(second);
    env.db.cleanup().await;
}

/// AC-HK-01: an agent that does not trust its file reports its state, and the control plane offers
/// no hook of it.
#[tokio::test]
async fn ac_hk_01_an_insecure_allow_list_gives_no_hook() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut insecure = report(&[("pg-dump", pg_dump())]);
    insecure.state = HookAllowListState::Insecure;
    let agent = env.connect_agent_reporting("host-bad", insecure).await;
    let (status, catalogue) = env.call(&env.viewer, "GET", "/api/v1/hooks", None).await;
    assert_eq!(status, StatusCode::OK, "{catalogue}");
    assert_eq!(catalogue["hooks"], json!([]));
    assert_eq!(catalogue["agents"][0]["hook_allow_list"], "insecure");
    let repository = env.create_repository(agent.id, "pw").await;
    let (status, problem) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/profiles",
            Some(profile_body(&repository, "no-hooks", before(json!([])))),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "hooks[0].hook_id");
    assert_eq!(problem["hook_id"], "pg-dump");
    env.db.cleanup().await;
}

/// AC-HK-05: the control plane does not save a profile with a hook that a target agent lacks, and
/// the error names the agent and the hook.
#[tokio::test]
async fn ac_hk_05_a_profile_needs_the_hook_on_every_target_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let with_hook = env
        .connect_agent_reporting("host-with", report(&[("pg-dump", pg_dump())]))
        .await;
    let without = env.connect_agent("host-without").await;
    let repository = env.create_repository(with_hook.id, "pw").await;

    // No plan yet: the hook exists on one active agent, so the profile is saved.
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, without.id).await;
    let _ = plan;
    let (status, problem) = env
        .call(
            &env.admin,
            "PATCH",
            &format!("/api/v1/profiles/{profile}"),
            Some(json!({
                "repository_id": repository.to_string(),
                "paths": {"all": ["/srv"]},
                "hooks": before(json!([{"name": "database", "value": "app"}])),
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "hooks[0].hook_id");
    assert_eq!(problem["reason"], "unknown");
    assert_eq!(problem["agent_id"], without.id.to_string());
    assert_eq!(problem["agent_name"], "host-without");
    assert_eq!(problem["hook_id"], "pg-dump");
    let (_, stored) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/profiles/{profile}"),
            None,
        )
        .await;
    assert_eq!(stored["hooks"], json!([]), "the profile is unchanged");

    // A profile with the hook cannot be bound to the agent that lacks it either.
    let hooked = env
        .create_profile(
            repository,
            json!({"hooks": before(json!([{"name": "database", "value": "app"}]))}),
        )
        .await;
    let (status, problem) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/plans",
            Some(json!({
                "name": "bad-plan",
                "profile_id": hooked.to_string(),
                "agent_id": without.id.to_string(),
                "cron": "0 2 1 1 *",
                "time_zone": "UTC",
                "missed_run_policy": "skip",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["agent_name"], "host-without");
    assert_eq!(problem["hook_id"], "pg-dump");

    // A hook that no agent reports is refused without an agent.
    let (status, problem) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/profiles",
            Some(profile_body(
                &repository,
                "ghost",
                json!([{"hook_id": "ghost", "phase": "before", "parameters": []}]),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["hook_id"], "ghost");
    assert!(problem.get("agent_id").is_none());
    env.db.cleanup().await;
}

/// FR-HK-03: the values of a profile must obey the schema that the agent reports.
#[tokio::test]
async fn fr_hk_03_the_values_obey_the_reported_schema() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let repository = env.create_repository(agent.id, "pw").await;
    let cases = [
        (
            "pattern",
            json!([{"name": "database", "value": "app; rm -rf /"}]),
            "hooks[0].parameters[0].value",
            "invalid_format",
        ),
        (
            "substitution",
            json!([{"name": "database", "value": "$(id)"}]),
            "hooks[0].parameters[0].value",
            "invalid_format",
        ),
        (
            "range",
            json!([{"name": "database", "value": "app"}, {"name": "jobs", "value": "9"}]),
            "hooks[0].parameters[1].value",
            "invalid_format",
        ),
        (
            "not a number",
            json!([{"name": "database", "value": "app"}, {"name": "jobs", "value": "4; id"}]),
            "hooks[0].parameters[1].value",
            "invalid_format",
        ),
        (
            "enum",
            json!([{"name": "database", "value": "app"}, {"name": "mode", "value": "slow"}]),
            "hooks[0].parameters[1].value",
            "invalid_format",
        ),
        (
            "unknown name",
            json!([{"name": "database", "value": "app"}, {"name": "other", "value": "x"}]),
            "hooks[0].parameters[1].name",
            "unknown",
        ),
        (
            "duplicate",
            json!([{"name": "database", "value": "a"}, {"name": "database", "value": "b"}]),
            "hooks[0].parameters[1].name",
            "unknown",
        ),
        (
            "missing required",
            json!([]),
            "hooks[0].parameters",
            "empty",
        ),
        (
            "control character",
            json!([{"name": "database", "value": "a\nb"}]),
            "hooks[0].parameters[0].value",
            "invalid_character",
        ),
        (
            "bad name",
            json!([{"name": "--database", "value": "app"}]),
            "hooks[0].parameters[0].name",
            "invalid_character",
        ),
    ];
    for (name, parameters, field, reason) in cases {
        let (status, problem) = env
            .call(
                &env.admin,
                "POST",
                "/api/v1/profiles",
                Some(profile_body(&repository, "invalid", before(parameters))),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {problem}");
        assert_eq!(problem["field"], field, "{name}: {problem}");
        assert_eq!(problem["reason"], reason, "{name}: {problem}");
    }
    // The hook runs in the phases of its entry only.
    let (status, problem) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/profiles",
            Some(profile_body(
                &repository,
                "phase",
                json!([{"hook_id": "pg-dump", "phase": "on_error",
                        "parameters": [{"name": "database", "value": "app"}]}]),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "hooks[0].phase");
    assert_eq!(problem["reason"], "unsupported");
    let valid = env
        .create_profile(
            repository,
            json!({"hooks": before(json!([
                {"name": "database", "value": "app"},
                {"name": "jobs", "value": "8"},
                {"name": "mode", "value": "fast"},
            ]))}),
        )
        .await;
    let (_, stored) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/profiles/{valid}"),
            None,
        )
        .await;
    assert_eq!(stored["hooks"][0]["parameters"][1]["value"], "8");
    env.db.cleanup().await;
}

/// ADR-0017: a secret parameter holds the identifier of a hook secret, never a value.
#[tokio::test]
async fn adr_0017_a_secret_parameter_is_a_reference_to_a_hook_secret() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let repository = env
        .create_repository(agent.id, "pw-of-the-repository")
        .await;
    let hook_secret = create_secret(&env, "dump.token", "hook_secret", CANARY).await;
    let other = create_secret(&env, "other.password", "repository_password", "x-other").await;

    for (name, value, reason) in [
        ("another purpose", other.as_str(), "unknown"),
        (
            "an unknown secret",
            "01890000-0000-7000-8000-000000000001",
            "unknown",
        ),
        ("a plain value", CANARY, "invalid_format"),
    ] {
        let (status, problem) = env
            .call(
                &env.admin,
                "POST",
                "/api/v1/profiles",
                Some(profile_body(
                    &repository,
                    "secret-bad",
                    before(json!([
                        {"name": "database", "value": "app"},
                        {"name": "token", "value": value},
                    ])),
                )),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {problem}");
        assert_eq!(problem["field"], "hooks[0].parameters[1].value", "{name}");
        assert_eq!(problem["reason"], reason, "{name}");
        assert!(!problem.to_string().contains(CANARY), "{name}");
    }
    let profile = env
        .create_profile(
            repository,
            json!({"hooks": before(json!([
                {"name": "database", "value": "app"},
                {"name": "token", "value": hook_secret},
            ]))}),
        )
        .await;
    let (_, view) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/profiles/{profile}"),
            None,
        )
        .await;
    assert_eq!(view["hooks"][0]["parameters"][1]["value"], hook_secret);
    assert!(!view.to_string().contains(CANARY));
    let mut tx = env.scoped().await;
    let audit: Vec<String> = sqlx::query_scalar("SELECT coalesce(diff::text, '') FROM audit_log")
        .fetch_all(&mut *tx)
        .await
        .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    assert!(audit.iter().all(|row| !row.contains(CANARY)));
    env.db.cleanup().await;
}

async fn secret_job(
    env: &Env,
    agent: &queue_support::FakeAgent,
    name: &str,
) -> (uuid::Uuid, String) {
    let repository = env
        .create_repository(agent.id, "pw-of-the-repository")
        .await;
    let secret = create_secret(env, name, "hook_secret", CANARY).await;
    let profile = env
        .create_profile(
            repository,
            json!({"hooks": before(json!([
                {"name": "database", "value": "app"},
                {"name": "token", "value": secret},
            ]))}),
        )
        .await;
    let plan = env.create_plan(profile, agent.id).await;
    (env.run_plan(plan).await, secret)
}

/// FR-HK-03, ADR-0017: the builder resolves the reference at dispatch time and records the
/// delivery.
#[tokio::test]
async fn adr_0017_the_builder_resolves_the_reference_and_audits_the_delivery() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let (_job, secret) = secret_job(&env, &agent, "dump.token").await;
    env.pass().await;
    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    assert!(matches!(job.steps[0].kind, Some(step::Kind::Hook(_))));
    let Some(step::Kind::Hook(hook)) = &job.steps[0].kind else {
        panic!("the first step is a hook");
    };
    assert_eq!(hook.hook_id, "pg-dump");
    let token = hook
        .parameters
        .iter()
        .find(|parameter| parameter.name == "token")
        .expect("the token parameter");
    assert_eq!(token.typed, Some(Typed::SecretValue(CANARY.to_owned())));
    assert!(matches!(job.steps[1].kind, Some(step::Kind::Backup(_))));
    // The record names the secret, never the value (AC-RP-03).
    let mut tx = env.scoped().await;
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT coalesce(diff::text, '') || ' ' || coalesce(object_id::text, '') \
         FROM audit_log WHERE action = 'secret_delivered'",
    )
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    assert!(rows.iter().any(|row| row.contains(&secret)), "{rows:?}");
    assert!(rows.iter().all(|row| !row.contains(CANARY)));
    env.db.cleanup().await;
}

/// AC-HK-05: the builder fails the job when the agent no longer reports the hook.
#[tokio::test]
async fn ac_hk_05_the_builder_refuses_a_hook_that_the_agent_no_longer_reports() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let (job, _) = secret_job(&env, &agent, "dump.token").await;
    // The administrator removed the hook and the agent restarted.
    let capabilities = restichub_proto::AgentCapability::ALL
        .iter()
        .map(|capability| capability.as_str().to_owned())
        .collect();
    let mut agent = env
        .reconnect_agent_reporting(&agent, capabilities, report(&[]))
        .await;
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["result_code"], "config_error");
    assert_eq!(body["result_message"], "hook_unavailable");
    assert!(
        agent
            .try_job(std::time::Duration::from_millis(300))
            .await
            .is_none()
    );
    env.db.cleanup().await;
}

/// AC-HK-05: the builder fails the job when the agent reports another schema.
#[tokio::test]
async fn ac_hk_05_the_builder_refuses_a_value_that_the_new_schema_rejects() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let (job, _) = secret_job(&env, &agent, "dump.token").await;
    let mut stricter = pg_dump();
    stricter.parameters.insert(
        "database".to_owned(),
        spec(
            ParameterKind::String {
                max_length: 63,
                pattern: Some("prod_[a-z]+".to_owned()),
            },
            true,
        ),
    );
    let capabilities = restichub_proto::AgentCapability::ALL
        .iter()
        .map(|capability| capability.as_str().to_owned())
        .collect();
    let _agent = env
        .reconnect_agent_reporting(&agent, capabilities, report(&[("pg-dump", stricter)]))
        .await;
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["result_message"], "hook_unavailable");
    env.db.cleanup().await;
}

/// FR-PR-09: an agent without the capability `hooks-v1` never gets a hook step.
#[tokio::test]
async fn fr_pr_09_an_agent_without_hooks_v1_never_gets_a_hook_step() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let capable = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let (job, _) = secret_job(&env, &capable, "dump.token").await;
    let mut downgraded = env.reconnect_agent(&capable, Vec::new()).await;
    // The row still holds the hooks of an earlier connection.
    common::seed_agent_hooks(
        env.db.database(),
        env.tenant,
        downgraded.id,
        &BTreeMap::from([("pg-dump".to_owned(), pg_dump())]),
        restichub_db::HookAllowListState::Loaded,
    )
    .await;
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["result_message"], "agent_capability_missing");
    assert!(
        downgraded
            .try_job(std::time::Duration::from_millis(300))
            .await
            .is_none()
    );
    env.db.cleanup().await;
}

/// AC-TN-01: a tenant sees the hooks of its own agents only.
#[tokio::test]
async fn ac_tn_01_the_catalogue_shows_the_hooks_of_the_own_tenant_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let _agent = env
        .connect_agent_reporting("host-a", report(&[("pg-dump", pg_dump())]))
        .await;
    let other = common::create_tenant_with_key(env.db.database(), "other-tenant").await;
    let token = common::seed_api_token(env.db.database(), other, "viewer").await;
    let (status, catalogue) = env.call(&token, "GET", "/api/v1/hooks", None).await;
    assert_eq!(status, StatusCode::OK, "{catalogue}");
    assert_eq!(catalogue["hooks"], json!([]));
    assert_eq!(catalogue["agents"], json!([]));
    // A profile of the other tenant cannot name the hook of the first tenant.
    let (status, _) = env
        .call(
            &token,
            "POST",
            "/api/v1/profiles",
            Some(json!({
                "name": "x",
                "repository_id": "01890000-0000-7000-8000-000000000001",
                "paths": {"all": ["/srv"]},
                "hooks": before(json!([])),
            })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    env.db.cleanup().await;
}

/// Section 2 of the requirements: every role may read the catalogue; no token gets 401.
#[tokio::test]
async fn the_catalogue_follows_the_role_matrix() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    for token in [&env.admin, &env.operator, &env.viewer] {
        let (status, _) = env.call(token, "GET", "/api/v1/hooks", None).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = env.call("not-a-token", "GET", "/api/v1/hooks", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    env.db.cleanup().await;
}
