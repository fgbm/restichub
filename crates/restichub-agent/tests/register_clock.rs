//! The `register` command against a control plane with a skewed clock (FR-IN-07).

#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::get;
use restichub_agent::cli::RegisterOptions;
use restichub_agent::commands;
use restichub_agent::enroll::ENROLL_PATH;
use restichub_agent::restic::Restic;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

struct FakeRestic;

impl Restic for FakeRestic {
    fn verify(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn version(&self) -> anyhow::Result<String> {
        Ok("0.19.1".to_owned())
    }
}

/// Answers a `GET` with a `Date` header that is `skew` ahead of the real time and counts `POST`.
fn skewed_server(skew: time::Duration, enrolls: Arc<AtomicUsize>) -> common::TestServer {
    let date = (OffsetDateTime::now_utc() + skew)
        .format(&Rfc2822)
        .expect("cannot format the date")
        .replace("+0000", "GMT");
    let app = Router::new().route(
        ENROLL_PATH,
        get(move || {
            let date = date.clone();
            async move {
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::DATE,
                    HeaderValue::from_str(&date).expect("a header value"),
                );
                (StatusCode::METHOD_NOT_ALLOWED, headers)
            }
        })
        .post(move || {
            enrolls.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::INTERNAL_SERVER_ERROR }
        }),
    );
    common::TestServer::start(app)
}

async fn register_against(server: &common::TestServer, name: &str) -> anyhow::Result<()> {
    let dir = common::temp_config_dir(name);
    let options = RegisterOptions {
        url: Some(server.url.clone()),
        token: Some("one-time-token".to_owned()),
        ca_file: Some(server.ca_file()),
        ..RegisterOptions::default()
    };
    commands::register_async(&options, &dir, &dir.with_extension("cache"), &FakeRestic).await
}

#[tokio::test(flavor = "multi_thread")]
async fn ac_in_02_a_skewed_clock_stops_the_registration_before_the_token_is_sent() {
    let enrolls = Arc::new(AtomicUsize::new(0));
    // The control plane is 20 minutes ahead, so the host clock is 20 minutes behind.
    let server = skewed_server(time::Duration::minutes(20), enrolls.clone());

    let error = register_against(&server, "clock-skewed")
        .await
        .expect_err("a skewed clock must stop the registration")
        .to_string();

    assert!(error.contains("the host clock is off by"), "{error}");
    assert!(error.contains("w32tm /resync"), "{error}");
    assert_eq!(
        enrolls.load(Ordering::SeqCst),
        0,
        "the token must stay unused"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ac_in_02_a_small_skew_lets_the_enrollment_request_go_out() {
    let enrolls = Arc::new(AtomicUsize::new(0));
    let server = skewed_server(time::Duration::minutes(2), enrolls.clone());

    let error = register_against(&server, "clock-small")
        .await
        .expect_err("the test server refuses the enrollment")
        .to_string();

    assert!(error.contains("refused the enrollment"), "{error}");
    assert!(error.contains("register again"), "{error}");
    assert_eq!(enrolls.load(Ordering::SeqCst), 1);
}
