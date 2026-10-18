//! The Prometheus endpoint (FR-MN-05, AC-MN-03).
//!
//! `promtool` is not a build dependency, so the test holds a strict reader of the text exposition
//! format: it checks the family declarations, the sample syntax, the label set and the histogram
//! invariants that `promtool check metrics` checks.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{StatusCode, header};
use http_body_util::BodyExt as _;
use restichub_proto::agent::v1::{AgentMessage, StepResult, StepStatus, agent_message};

use queue_support::{Env, eventually, test_queue};

#[derive(Debug)]
struct Sample {
    name: String,
    labels: BTreeMap<String, String>,
    value: f64,
}

struct Exposition {
    types: BTreeMap<String, String>,
    samples: Vec<Sample>,
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == ':')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// Reads the labels of `{a="b",c="d"}`. A quote or a backslash in a value is escaped.
fn parse_labels(text: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (name, after) = rest.split_once("=\"").expect("a label has a quoted value");
        assert!(valid_name(name) && !name.contains(':'), "label name {name}");
        let mut value = String::new();
        let mut chars = after.char_indices();
        let end = loop {
            let (index, character) = chars.next().expect("the label value ends");
            match character {
                '\\' => {
                    let (_, escaped) = chars.next().expect("an escape follows");
                    assert!(matches!(escaped, '\\' | '"' | 'n'), "escape {escaped}");
                    value.push(escaped);
                }
                '"' => break index,
                other => value.push(other),
            }
        };
        assert!(
            labels.insert(name.to_owned(), value).is_none(),
            "duplicate label {name}"
        );
        rest = &after[end + 1..];
        rest = rest.strip_prefix(',').unwrap_or(rest);
    }
    labels
}

fn parse(text: &str) -> Exposition {
    assert!(text.ends_with('\n'), "the text ends with a line break");
    let mut types = BTreeMap::new();
    let mut helped = BTreeSet::new();
    let mut samples = Vec::new();
    for line in text.lines() {
        assert!(!line.is_empty(), "no empty line");
        if let Some(rest) = line.strip_prefix("# HELP ") {
            let (name, help) = rest.split_once(' ').expect("a help text");
            assert!(valid_name(name) && !help.is_empty());
            assert!(helped.insert(name.to_owned()), "one HELP for {name}");
        } else if let Some(rest) = line.strip_prefix("# TYPE ") {
            let (name, kind) = rest.split_once(' ').expect("a type");
            assert!(valid_name(name));
            assert!(
                ["counter", "gauge", "histogram", "summary", "untyped"].contains(&kind),
                "type {kind}"
            );
            assert!(
                types.insert(name.to_owned(), kind.to_owned()).is_none(),
                "one TYPE for {name}"
            );
        } else {
            assert!(!line.starts_with('#'), "unknown comment {line}");
            let (series, value) = line.rsplit_once(' ').expect("a value");
            let value: f64 = value.parse().expect("a numeric value");
            let (name, labels) = match series.split_once('{') {
                Some((name, rest)) => (
                    name,
                    parse_labels(rest.strip_suffix('}').expect("a closing brace")),
                ),
                None => (series, BTreeMap::new()),
            };
            assert!(valid_name(name), "metric name {name}");
            samples.push(Sample {
                name: name.to_owned(),
                labels,
                value,
            });
        }
    }
    // Every sample belongs to a declared family.
    for sample in &samples {
        let family = ["_bucket", "_sum", "_count"]
            .iter()
            .find_map(|suffix| {
                sample
                    .name
                    .strip_suffix(suffix)
                    .filter(|base| types.get(*base).is_some_and(|kind| kind == "histogram"))
            })
            .unwrap_or(&sample.name);
        assert!(types.contains_key(family), "no TYPE for {}", sample.name);
        assert!(helped.contains(family), "no HELP for {}", sample.name);
    }
    // Counters end with _total.
    for (name, kind) in &types {
        if kind == "counter" {
            assert!(name.ends_with("_total"), "counter {name}");
        }
    }
    Exposition { types, samples }
}

type Series = Vec<(String, String)>;

fn check_histograms(exposition: &Exposition, family: &str) {
    let mut groups: BTreeMap<Series, Vec<(f64, f64)>> = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for sample in &exposition.samples {
        let mut labels: Vec<_> = sample
            .labels
            .iter()
            .filter(|(name, _)| *name != "le")
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        labels.sort();
        if sample.name == format!("{family}_bucket") {
            let bound = match sample.labels["le"].as_str() {
                "+Inf" => f64::INFINITY,
                other => other.parse().expect("a numeric bound"),
            };
            groups
                .entry(labels)
                .or_default()
                .push((bound, sample.value));
        } else if sample.name == format!("{family}_count") {
            counts.insert(labels, sample.value);
        }
    }
    for (labels, buckets) in groups {
        assert!(
            buckets
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0 && pair[0].1 <= pair[1].1),
            "the buckets are sorted and cumulative: {buckets:?}"
        );
        let last = buckets.last().expect("a bucket");
        assert!(last.0.is_infinite(), "the last bucket is +Inf");
        assert_eq!(Some(&last.1), counts.get(&labels), "+Inf equals the count");
    }
}

async fn scrape(env: &Env, token: Option<&str>) -> (StatusCode, Option<String>, String) {
    let request = match token {
        Some(token) => common::bearer_request("GET", "/metrics", token, None),
        None => {
            let mut request = axum::http::Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(axum::body::Body::empty())
                .expect("the request");
            request.extensions_mut().insert(common::peer());
            request
        }
    };
    let response = common::send_response(&env.app, request).await;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).expect("UTF-8 text"),
    )
}

/// Pipes the exposition through the real `promtool check metrics` when the CI job provides it.
fn check_with_promtool(text: &str) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let Some(promtool) = std::env::var_os("RESTICHUB_TEST_PROMTOOL") else {
        assert!(
            std::env::var_os("RESTICHUB_REQUIRE_PROMTOOL").is_none(),
            "RESTICHUB_REQUIRE_PROMTOOL is set but RESTICHUB_TEST_PROMTOOL is not"
        );
        return;
    };
    let mut child = Command::new(promtool)
        .args(["check", "metrics"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start promtool");
    child
        .stdin
        .take()
        .expect("the stdin")
        .write_all(text.as_bytes())
        .expect("cannot write to promtool");
    let output = child.wait_with_output().expect("promtool must finish");
    assert!(
        output.status.success(),
        "promtool rejected the exposition: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn labelled<'a>(exposition: &'a Exposition, name: &'a str) -> impl Iterator<Item = &'a Sample> {
    exposition
        .samples
        .iter()
        .filter(move |sample| sample.name == name)
}

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
