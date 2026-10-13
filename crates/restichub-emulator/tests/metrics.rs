//! Metrics: histogram, Prometheus text endpoint and JSON summary (NFR-PERF-04).
#![allow(clippy::expect_used)]

use std::sync::Arc;

use restichub_emulator::metrics::{Metrics, bind, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;

async fn http_get(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    response
}

#[test]
fn the_summary_holds_the_percentiles_and_the_counts() {
    let metrics = Metrics::default();
    for value in 1..=100 {
        metrics.record_connect(value);
        metrics.record_ack(value * 10);
        metrics.record_result(value * 100);
    }
    metrics.inc_reconnect();
    metrics.inc_rejected();

    let summary = metrics.summary(100, 10.0);

    assert_eq!(summary.agents, 100);
    assert_eq!(summary.connects_total, 100);
    assert_eq!(summary.reconnects_total, 1);
    assert_eq!(summary.rejected_total, 1);
    assert_eq!(summary.jobs_acked_total, 100);
    assert_eq!(summary.results_total, 100);
    assert_eq!(summary.connect_seconds.count, 100);
    assert!(summary.connect_seconds.p50 <= summary.connect_seconds.p95);
    assert!(summary.connect_seconds.p95 <= summary.connect_seconds.p99);
    assert!(summary.connect_seconds.max >= summary.connect_seconds.p99);
}

#[test]
fn the_summary_serializes_to_the_expected_fields() {
    let metrics = Metrics::default();
    metrics.record_connect(1_000);
    let summary = metrics.summary(1, 5.0);

    let json = serde_json::to_value(&summary).expect("serialize");
    for field in [
        "agents",
        "duration_seconds",
        "connects_total",
        "reconnects_total",
        "rejected_total",
        "jobs_acked_total",
        "results_total",
        "streams_current",
        "connect_seconds",
        "ack_millis",
        "result_millis",
    ] {
        assert!(json.get(field).is_some(), "missing field {field}");
    }
    for field in ["count", "p50", "p95", "p99", "max"] {
        assert!(
            json["connect_seconds"].get(field).is_some(),
            "missing latency field {field}"
        );
    }
}

#[test]
fn the_prometheus_text_has_the_metric_types() {
    let metrics = Metrics::default();
    metrics.record_connect(2_000);
    metrics.record_ack(500);
    metrics.inc_reconnect();

    let text = metrics.render_prometheus();

    assert!(text.contains("# TYPE restichub_emulator_connect_seconds summary"));
    assert!(text.contains("restichub_emulator_connect_seconds{quantile=\"0.95\"}"));
    assert!(text.contains("# TYPE restichub_emulator_connects_total counter"));
    assert!(text.contains("restichub_emulator_connects_total 1"));
    assert!(text.contains("restichub_emulator_reconnects_total 1"));
    assert!(text.contains("# TYPE restichub_emulator_streams gauge"));
    assert!(text.contains("restichub_emulator_streams 1"));
}

#[tokio::test]
async fn the_endpoint_serves_the_prometheus_text() {
    let metrics = Arc::new(Metrics::default());
    metrics.record_connect(2_000);
    let listener = bind("127.0.0.1:0".parse().expect("address"))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local address");
    let (shutdown, receiver) = watch::channel(false);
    let server = tokio::spawn(serve(listener, Arc::clone(&metrics), receiver));

    let response = http_get(addr, "/metrics").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("restichub_emulator_connects_total 1"),
        "{response}"
    );

    let missing = http_get(addr, "/nope").await;
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

    let _ = shutdown.send(true);
    server.await.expect("join").expect("serve");
}
