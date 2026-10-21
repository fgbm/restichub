//! The Prometheus endpoint (FR-MN-05, AC-MN-03).

#![allow(clippy::expect_used)]

mod common;
mod metrics_support;
mod queue_support;

use std::collections::BTreeMap;

use axum::http::StatusCode;
use restichub_cp::config::TenancyConfig;
use restichub_cp::listener_stats::RefusalReason;
use restichub_proto::agent::v1::{AgentMessage, StepResult, StepStatus, agent_message};

use metrics_support::{check_histograms, check_with_promtool, labelled, parse, scrape, scrape_app};
use queue_support::{Env, eventually, test_queue};

#[tokio::test]
async fn ac_mn_03_the_metrics_endpoint_passes_a_strict_format_check() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("metrics-agent").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(serde_json::json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let id = queue_support::decode_job_id(&signed);
    agent.start(&env, &id).await;
    // The job runs for five seconds of the manual clock.
    env.clock.advance(5);
    agent
        .tx
        .send(AgentMessage {
            payload: Some(agent_message::Payload::StepResult(StepResult {
                job_id: id.clone(),
                step_id: "backup".to_owned(),
                status: StepStatus::Success as i32,
                ..StepResult::default()
            })),
        })
        .await
        .expect("cannot send the step result");
    agent.finish(&env, &id).await;
    eventually(|| async { env.job_status(job).await == "succeeded" }).await;

    let (status, content_type, text) = scrape(&env, Some(&env.viewer)).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        content_type.as_deref(),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    check_with_promtool(&text);
    let exposition = parse(&text);
    for family in [
        "restichub_last_success_timestamp_seconds",
        "restichub_job_duration_seconds",
        "restichub_job_dispatch_delay_seconds",
        "restichub_agent_up",
        "restichub_repo_lock_held",
        "restichub_jobs_total",
    ] {
        assert!(exposition.types.contains_key(family), "{family}");
    }
    // Each sample has the label `tenant`.
    assert!(!exposition.samples.is_empty());
    for sample in &exposition.samples {
        assert_eq!(
            sample.labels.get("tenant").map(String::as_str),
            Some(env.tenant.to_string().as_str()),
            "{sample:?}"
        );
    }
    check_histograms(&exposition, "restichub_job_duration_seconds");
    check_histograms(&exposition, "restichub_job_dispatch_delay_seconds");
    let sent = labelled(&exposition, "restichub_job_dispatch_delay_seconds_count")
        .next()
        .expect("the dispatch delay count");
    assert_eq!(sent.value, 1.0);

    let last = labelled(&exposition, "restichub_last_success_timestamp_seconds")
        .next()
        .expect("the last success of the plan");
    assert_eq!(last.labels["plan"], plan.to_string());
    assert_eq!(last.value as i64, env.now().unix_timestamp());

    let up = labelled(&exposition, "restichub_agent_up")
        .find(|sample| sample.labels["agent"] == agent.id.to_string())
        .expect("the agent");
    assert_eq!(up.value, 1.0);

    let lock = labelled(&exposition, "restichub_repo_lock_held")
        .find(|sample| sample.labels["repository"] == repository.to_string())
        .expect("the repository");
    assert_eq!(lock.value, 0.0);

    let duration_sum = labelled(&exposition, "restichub_job_duration_seconds_sum")
        .find(|sample| sample.labels["kind"] == "backup")
        .expect("the backup duration");
    assert_eq!(duration_sum.value, 5.0);
    let five = labelled(&exposition, "restichub_job_duration_seconds_bucket")
        .find(|sample| sample.labels["kind"] == "backup" && sample.labels["le"] == "5")
        .expect("the five second bucket");
    assert_eq!(five.value, 1.0);
    let one = labelled(&exposition, "restichub_job_duration_seconds_bucket")
        .find(|sample| sample.labels["kind"] == "backup" && sample.labels["le"] == "1")
        .expect("the one second bucket");
    assert_eq!(one.value, 0.0);

    let succeeded = labelled(&exposition, "restichub_jobs_total")
        .find(|sample| sample.labels["status"] == "succeeded")
        .expect("the succeeded count");
    assert_eq!(succeeded.value, 1.0);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_the_metrics_show_a_held_repository_lock() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("lock-agent").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(serde_json::json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let id = queue_support::decode_job_id(&signed);
    agent.start(&env, &id).await;
    agent.lock_held(&id).await;
    eventually(|| async { env.plan_jobs(plan).await.len() == 2 }).await;

    let (_, _, text) = scrape(&env, Some(&env.viewer)).await;
    let exposition = parse(&text);
    let lock = labelled(&exposition, "restichub_repo_lock_held")
        .find(|sample| sample.labels["repository"] == repository.to_string())
        .expect("the repository");
    assert_eq!(lock.value, 1.0, "{text}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_the_metrics_show_the_samples_of_the_own_tenant_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("own-agent").await;
    let other = common::create_tenant_with_key(env.db.database(), "metrics-other").await;
    let other_viewer = common::seed_api_token(env.db.database(), other, "viewer").await;

    let (status, _, text) = scrape(&env, Some(&other_viewer)).await;
    assert_eq!(status, StatusCode::OK);
    let exposition = parse(&text);
    assert!(
        !text.contains(&agent.id.to_string()),
        "the scrape of another tenant names the agent"
    );
    for sample in &exposition.samples {
        assert_eq!(sample.labels["tenant"], other.to_string());
    }
    // The listener serves every tenant of a multi-tenant installation.
    assert!(!text.contains("restichub_agent_connections_refused_total"));
    assert!(!text.contains("restichub_agent_accept_errors_total"));
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_a_single_tenant_installation_shows_the_listener_refusals() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let single = env
        .state
        .clone()
        .with_tenancy(TenancyConfig::single("queue-tenant"));
    single.listener_stats().record_refusal(RefusalReason::Cap);
    let app = common::api_router(single);

    let (status, _, text) = scrape_app(&app, Some(&env.viewer)).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    check_with_promtool(&text);
    let exposition = parse(&text);
    assert_eq!(
        exposition
            .types
            .get("restichub_agent_connections_refused_total")
            .map(String::as_str),
        Some("counter")
    );
    let refused: BTreeMap<_, _> =
        labelled(&exposition, "restichub_agent_connections_refused_total")
            .map(|sample| (sample.labels["reason"].clone(), sample.value))
            .collect();
    assert_eq!(refused.len(), RefusalReason::ALL.len());
    assert_eq!(refused["cap"], 1.0);
    assert_eq!(refused["handshakes"], 0.0);
    let errors = labelled(&exposition, "restichub_agent_accept_errors_total")
        .next()
        .expect("the accept error count");
    assert_eq!(errors.value, 0.0);
    for sample in &exposition.samples {
        assert_eq!(sample.labels["tenant"], env.tenant.to_string());
    }
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_the_metrics_endpoint_needs_a_credential() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (status, _, _) = scrape(&env, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = scrape(&env, Some("not-a-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    env.db.cleanup().await;
}

/// Gives each seeded job an end status and a duration in seconds.
async fn seed_durations(
    env: &Env,
    agent: restichub_db::AgentId,
    repository: uuid::Uuid,
    secs: &[f64],
) {
    let repository = restichub_db::RepositoryId::from_uuid(repository);
    for seconds in secs {
        let job = common::seed_job(env.db.database(), env.tenant, agent, repository).await;
        let mut tx = env.db.owner_pool().begin().await.expect("the transaction");
        sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
            .bind(env.tenant.to_string())
            .execute(&mut *tx)
            .await
            .expect("the tenant scope");
        sqlx::query(
            "UPDATE jobs SET status = 'succeeded', started_at = now() - interval '1 day', \
             finished_at = now() - interval '1 day' + make_interval(secs => $2) WHERE id = $1",
        )
        .bind(job)
        .bind(seconds)
        .execute(&mut *tx)
        .await
        .expect("the update");
        tx.commit().await.expect("the commit");
    }
}

#[tokio::test]
async fn ac_mn_03_the_histogram_buckets_count_each_job_once_for_every_bound() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("histogram-agent").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let durations = [0.5, 1.0, 4.0, 15.0, 61.0, 299.0, 4000.0, 30.0, 0.0];
    seed_durations(&env, agent.id, repository, &durations).await;

    let (status, _, text) = scrape(&env, Some(&env.viewer)).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let exposition = parse(&text);
    check_histograms(&exposition, "restichub_job_duration_seconds");
    let bucket = |le: &str| -> f64 {
        labelled(&exposition, "restichub_job_duration_seconds_bucket")
            .find(|sample| sample.labels["kind"] == "test_access" && sample.labels["le"] == le)
            .unwrap_or_else(|| panic!("the bucket {le}"))
            .value
    };
    for bound in restichub_db::metrics::DURATION_BUCKETS {
        let expected = durations.iter().filter(|secs| **secs <= bound).count() as f64;
        assert_eq!(bucket(&bound.to_string()), expected, "le={bound}");
    }
    assert_eq!(bucket("+Inf"), durations.len() as f64);
    let sum = labelled(&exposition, "restichub_job_duration_seconds_sum")
        .find(|sample| sample.labels["kind"] == "test_access")
        .expect("the sum");
    assert!(
        (sum.value - durations.iter().sum::<f64>()).abs() < 0.01,
        "{}",
        sum.value
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_a_snapshot_is_reused_for_fifteen_seconds() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("cache-agent").await;
    let repository = env.create_repository(agent.id, "pw").await;

    let succeeded = |text: &str| -> f64 {
        let exposition = parse(text);
        labelled(&exposition, "restichub_jobs_total")
            .find(|sample| sample.labels["status"] == "succeeded")
            .expect("the succeeded count")
            .value
    };
    let (_, _, first) = scrape(&env, Some(&env.viewer)).await;
    assert_eq!(succeeded(&first), 0.0);
    seed_durations(&env, agent.id, repository, &[2.0]).await;
    env.clock.advance(14);
    let (_, _, cached) = scrape(&env, Some(&env.viewer)).await;
    assert_eq!(succeeded(&cached), 0.0, "the snapshot is reused");
    env.clock.advance(2);
    let (_, _, fresh) = scrape(&env, Some(&env.viewer)).await;
    assert_eq!(
        succeeded(&fresh),
        1.0,
        "the snapshot is refreshed after 15 seconds"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_a_platform_admin_session_gets_no_metrics() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let admin = common::create_platform_admin(env.db.database(), "metrics-platform", "phc").await;
    let cookie =
        common::session_cookie(env.db.database(), restichub_db::PLATFORM_TENANT_ID, admin).await;
    let response = common::send_response(
        &env.app,
        common::request("GET", "/metrics", Some(&cookie), None),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    env.db.cleanup().await;
}
