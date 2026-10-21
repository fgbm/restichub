//! Agent-side metrics of a load run (NFR-PERF-04).
//!
//! The emulator records the connect time, the job acknowledgement latency, the
//! result latency and the counters of a run. The counters render as a Prometheus
//! text endpoint and as a JSON summary. The endpoint is the scrape source of
//! bench S2 (#37); the summary goes into the acceptance report of AC-NFR-01
//! (#38). The control plane CPU, RAM and dispatch delay are not here: they come
//! from the control plane side of bench S2.

use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use anyhow::Context as _;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::lock;

/// Upper bounds in microseconds of the fixed histogram buckets.
///
/// The bounds grow with the magnitude, so a small latency keeps a fine
/// resolution and a slow one stays in a bounded footprint. A percentile is the
/// upper bound of its bucket, never above the largest sample.
const BUCKET_BOUNDS_US: &[u64] = &[
    50,
    100,
    250,
    500,
    750,
    1_000,
    2_500,
    5_000,
    7_500,
    10_000,
    25_000,
    50_000,
    75_000,
    100_000,
    250_000,
    500_000,
    750_000,
    1_000_000,
    2_500_000,
    5_000_000,
    7_500_000,
    10_000_000,
    30_000_000,
    60_000_000,
    120_000_000,
    300_000_000,
    u64::MAX,
];

/// Fixed buckets of microseconds for one latency.
///
/// The crate avoids a histogram dependency: the workspace lints refuse
/// `unwrap` and `expect`, and every constructor of `hdrhistogram` returns a
/// `Result`. A fixed bucket set gives the p50, p95 and p99 of a bench run with
/// no dependency and a fixed footprint.
#[derive(Debug, Clone)]
pub struct Histogram {
    counts: Vec<u64>,
    count: u64,
    sum: u64,
    max: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            counts: vec![0; BUCKET_BOUNDS_US.len()],
            count: 0,
            sum: 0,
            max: 0,
        }
    }
}

impl Histogram {
    /// Adds one sample in microseconds.
    pub fn record(&mut self, microseconds: u64) {
        let index = BUCKET_BOUNDS_US
            .partition_point(|bound| *bound < microseconds)
            .min(self.counts.len() - 1);
        self.counts[index] += 1;
        self.count += 1;
        self.sum = self.sum.saturating_add(microseconds);
        self.max = self.max.max(microseconds);
    }

    /// Upper bound in microseconds of the `quantile` (0.0 to 1.0).
    ///
    /// Zero when the histogram holds no sample.
    #[must_use]
    pub fn value_at_quantile(&self, quantile: f64) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let rank = (quantile.clamp(0.0, 1.0) * self.count as f64).ceil() as u64;
        let rank = rank.max(1);
        let mut cumulative = 0u64;
        for (index, count) in self.counts.iter().enumerate() {
            cumulative += count;
            if cumulative >= rank {
                return BUCKET_BOUNDS_US[index].min(self.max);
            }
        }
        self.max
    }
}

/// Counters and histograms of one run.
#[derive(Debug, Default)]
pub struct Metrics {
    connect_seconds: Mutex<Histogram>,
    ack_micros: Mutex<Histogram>,
    result_micros: Mutex<Histogram>,
    connects: AtomicU64,
    reconnects: AtomicU64,
    rejected: AtomicU64,
    jobs_acked: AtomicU64,
    results: AtomicU64,
    streams: AtomicI64,
    log_chunks_sent: AtomicU64,
    log_bytes_sent: AtomicU64,
    log_chunks_acked: AtomicU64,
    log_chunks_resent: AtomicU64,
    log_chunks_unacked: AtomicI64,
    forged_results_foreign: AtomicU64,
    forged_results_unknown: AtomicU64,
    flood_messages: AtomicU64,
    oversize_messages: AtomicU64,
}

impl Metrics {
    /// Adds a connect time in microseconds and counts the new stream.
    pub fn record_connect(&self, microseconds: u64) {
        lock(&self.connect_seconds).record(microseconds);
        self.connects.fetch_add(1, Ordering::Relaxed);
        self.streams.fetch_add(1, Ordering::Relaxed);
    }

    /// Removes one open stream.
    pub fn stream_closed(&self) {
        self.streams.fetch_sub(1, Ordering::Relaxed);
    }

    /// Counts one connection attempt after a dropped stream or a failed attempt.
    pub fn inc_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts one job that the agent refused.
    pub fn inc_rejected(&self) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds the latency from the job receipt to the JobAck, in microseconds.
    pub fn record_ack(&self, microseconds: u64) {
        lock(&self.ack_micros).record(microseconds);
        self.jobs_acked.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds the latency from the JobAck to the JobResult, in microseconds.
    pub fn record_result(&self, microseconds: u64) {
        lock(&self.result_micros).record(microseconds);
        self.results.fetch_add(1, Ordering::Relaxed);
    }

    pub fn log_chunk_sent(&self, bytes: usize) {
        self.log_chunks_sent.fetch_add(1, Ordering::Relaxed);
        self.log_bytes_sent
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Counts chunks that went into a spool.
    pub fn log_chunks_kept(&self, count: usize) {
        self.log_chunks_unacked
            .fetch_add(count as i64, Ordering::Relaxed);
    }

    /// Counts chunks that a `LogAck` removed from a spool.
    pub fn log_chunks_acked(&self, count: usize) {
        self.log_chunks_acked
            .fetch_add(count as u64, Ordering::Relaxed);
        self.log_chunks_unacked
            .fetch_sub(count as i64, Ordering::Relaxed);
    }

    /// Counts chunks that left a spool without an acknowledgement.
    pub fn log_chunks_dropped(&self, count: usize) {
        self.log_chunks_unacked
            .fetch_sub(count as i64, Ordering::Relaxed);
    }

    /// Counts chunks sent again on a new stream.
    pub fn log_chunks_resent(&self, count: usize) {
        self.log_chunks_resent
            .fetch_add(count as u64, Ordering::Relaxed);
    }

    /// Counts one forged job result; `foreign` when it names a job of another agent.
    pub fn forged_result(&self, foreign: bool) {
        let counter = if foreign {
            &self.forged_results_foreign
        } else {
            &self.forged_results_unknown
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn flood_message(&self) {
        self.flood_messages.fetch_add(1, Ordering::Relaxed);
    }

    pub fn oversize_message(&self) {
        self.oversize_messages.fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn logs(&self) -> LogSummary {
        LogSummary {
            chunks_sent: self.log_chunks_sent.load(Ordering::Relaxed),
            bytes_sent: self.log_bytes_sent.load(Ordering::Relaxed),
            chunks_acked: self.log_chunks_acked.load(Ordering::Relaxed),
            chunks_resent: self.log_chunks_resent.load(Ordering::Relaxed),
            chunks_unacked: self.log_chunks_unacked.load(Ordering::Relaxed),
        }
    }

    #[must_use]
    pub fn hostile(&self) -> HostileSummary {
        HostileSummary {
            forged_results_foreign: self.forged_results_foreign.load(Ordering::Relaxed),
            forged_results_unknown: self.forged_results_unknown.load(Ordering::Relaxed),
            flood_messages: self.flood_messages.load(Ordering::Relaxed),
            oversize_messages: self.oversize_messages.load(Ordering::Relaxed),
        }
    }

    #[must_use]
    pub fn connects(&self) -> u64 {
        self.connects.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn reconnects(&self) -> u64 {
        self.reconnects.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn jobs_acked(&self) -> u64 {
        self.jobs_acked.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn results(&self) -> u64 {
        self.results.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn streams(&self) -> i64 {
        self.streams.load(Ordering::Relaxed)
    }

    /// Writes the counters in the Prometheus text format.
    #[must_use]
    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();
        write_summary(
            &mut out,
            "restichub_emulator_connect_seconds",
            "Seconds from the agent start to the established stream.",
            &lock(&self.connect_seconds),
        );
        write_summary(
            &mut out,
            "restichub_emulator_ack_seconds",
            "Seconds from the job receipt to the JobAck.",
            &lock(&self.ack_micros),
        );
        write_summary(
            &mut out,
            "restichub_emulator_result_seconds",
            "Seconds from the JobAck to the JobResult.",
            &lock(&self.result_micros),
        );
        write_counter(
            &mut out,
            "restichub_emulator_connects_total",
            "Established streams since the start.",
            self.connects(),
        );
        write_counter(
            &mut out,
            "restichub_emulator_reconnects_total",
            "Connection attempts after a dropped stream or a failed attempt.",
            self.reconnects(),
        );
        write_counter(
            &mut out,
            "restichub_emulator_rejected_total",
            "Jobs that the agents refused.",
            self.rejected(),
        );
        write_counter(
            &mut out,
            "restichub_emulator_jobs_acked_total",
            "Jobs acknowledged.",
            self.jobs_acked(),
        );
        write_counter(
            &mut out,
            "restichub_emulator_results_total",
            "Job results reported.",
            self.results(),
        );
        write_gauge(
            &mut out,
            "restichub_emulator_streams",
            "Agent streams open now.",
            self.streams(),
        );
        let logs = self.logs();
        for (name, help, value) in [
            (
                "restichub_emulator_log_chunks_total",
                "Log chunks sent.",
                logs.chunks_sent,
            ),
            (
                "restichub_emulator_log_bytes_total",
                "Log bytes sent.",
                logs.bytes_sent,
            ),
            (
                "restichub_emulator_log_chunks_acked_total",
                "Log chunks that a LogAck removed from a spool.",
                logs.chunks_acked,
            ),
            (
                "restichub_emulator_log_chunks_resent_total",
                "Log chunks sent again on a new stream.",
                logs.chunks_resent,
            ),
        ] {
            write_counter(&mut out, name, help, value);
        }
        write_gauge(
            &mut out,
            "restichub_emulator_log_chunks_unacked",
            "Log chunks that wait for a LogAck.",
            logs.chunks_unacked,
        );
        let hostile = self.hostile();
        for (name, help, value) in [
            (
                "restichub_emulator_forged_results_foreign_total",
                "Forged job results that name a job of another agent.",
                hostile.forged_results_foreign,
            ),
            (
                "restichub_emulator_forged_results_unknown_total",
                "Forged job results that name a job that does not exist.",
                hostile.forged_results_unknown,
            ),
            (
                "restichub_emulator_flood_messages_total",
                "Flood messages of the hostile agents.",
                hostile.flood_messages,
            ),
            (
                "restichub_emulator_oversize_messages_total",
                "Oversize messages of the hostile agents.",
                hostile.oversize_messages,
            ),
        ] {
            write_counter(&mut out, name, help, value);
        }
        out
    }

    /// Collects the counters and the percentiles of a run.
    #[must_use]
    pub fn summary(&self, agents: usize, duration_seconds: f64) -> Summary {
        Summary {
            agents,
            duration_seconds,
            connects_total: self.connects(),
            reconnects_total: self.reconnects(),
            rejected_total: self.rejected(),
            jobs_acked_total: self.jobs_acked(),
            results_total: self.results(),
            streams_current: self.streams(),
            connect_seconds: latency(&lock(&self.connect_seconds), 1e-6),
            ack_millis: latency(&lock(&self.ack_micros), 1e-3),
            result_millis: latency(&lock(&self.result_micros), 1e-3),
            logs: self.logs(),
            hostile: self.hostile(),
        }
    }
}

/// The log counters of a run (FR-AG-08, FR-LG-07).
#[derive(Debug, Clone, Serialize)]
pub struct LogSummary {
    pub chunks_sent: u64,
    pub bytes_sent: u64,
    pub chunks_acked: u64,
    pub chunks_resent: u64,
    pub chunks_unacked: i64,
}

/// The messages of the hostile agents, which the control plane must refuse.
#[derive(Debug, Clone, Serialize)]
pub struct HostileSummary {
    pub forged_results_foreign: u64,
    pub forged_results_unknown: u64,
    pub flood_messages: u64,
    pub oversize_messages: u64,
}

/// The counts and the percentiles of one latency metric.
#[derive(Debug, Clone, Serialize)]
pub struct LatencySummary {
    pub count: u64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Counts and percentiles of one run.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub agents: usize,
    pub duration_seconds: f64,
    pub connects_total: u64,
    pub reconnects_total: u64,
    pub rejected_total: u64,
    pub jobs_acked_total: u64,
    pub results_total: u64,
    pub streams_current: i64,
    pub connect_seconds: LatencySummary,
    pub ack_millis: LatencySummary,
    pub result_millis: LatencySummary,
    pub logs: LogSummary,
    pub hostile: HostileSummary,
}

fn latency(histogram: &Histogram, scale: f64) -> LatencySummary {
    LatencySummary {
        count: histogram.count,
        p50: histogram.value_at_quantile(0.50) as f64 * scale,
        p95: histogram.value_at_quantile(0.95) as f64 * scale,
        p99: histogram.value_at_quantile(0.99) as f64 * scale,
        max: histogram.max as f64 * scale,
    }
}

fn write_summary(out: &mut String, name: &str, help: &str, histogram: &Histogram) {
    out.push_str("# HELP ");
    out.push_str(name);
    out.push(' ');
    out.push_str(help);
    out.push_str("\n# TYPE ");
    out.push_str(name);
    out.push_str(" summary\n");
    for (quantile, value) in [
        ("0.5", histogram.value_at_quantile(0.50)),
        ("0.95", histogram.value_at_quantile(0.95)),
        ("0.99", histogram.value_at_quantile(0.99)),
    ] {
        out.push_str(&format!(
            "{name}{{quantile=\"{quantile}\"}} {}\n",
            seconds(value)
        ));
    }
    out.push_str(&format!("{name}_sum {}\n", seconds(histogram.sum)));
    out.push_str(&format!("{name}_count {}\n", histogram.count));
}

fn write_counter(out: &mut String, name: &str, help: &str, value: u64) {
    out.push_str(&format!(
        "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
    ));
}

fn write_gauge(out: &mut String, name: &str, help: &str, value: i64) {
    out.push_str(&format!(
        "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}\n"
    ));
}

fn seconds(microseconds: u64) -> f64 {
    microseconds as f64 / 1_000_000.0
}

/// Serves the Prometheus endpoint on `listener` until `shutdown` is set.
///
/// # Errors
///
/// Returns an error when an accept or a write fails.
pub async fn serve(
    listener: TcpListener,
    metrics: std::sync::Arc<Metrics>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("cannot accept a metrics connection")?;
                let metrics = std::sync::Arc::clone(&metrics);
                tokio::spawn(async move {
                    if let Err(error) = respond(stream, &metrics).await {
                        eprintln!("metrics connection: {error:#}");
                    }
                });
            }
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn respond(mut stream: TcpStream, metrics: &Metrics) -> anyhow::Result<()> {
    let mut buffer = [0u8; 1024];
    let read = stream
        .read(&mut buffer)
        .await
        .context("cannot read the request")?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let (status, body) = if path == "/metrics" || path == "/" {
        ("200 OK", metrics.render_prometheus())
    } else {
        ("404 Not Found", String::from("not found\n"))
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .context("cannot write the response")?;
    stream.flush().await.context("cannot flush the response")?;
    Ok(())
}

/// Opens a listener for the metrics endpoint.
///
/// # Errors
///
/// Returns an error when the address is in use or not local.
pub async fn bind(addr: SocketAddr) -> anyhow::Result<TcpListener> {
    TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind the metrics endpoint on {addr}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_histogram_reports_zero() {
        let histogram = Histogram::default();

        assert_eq!(histogram.value_at_quantile(0.95), 0);
    }

    #[test]
    fn percentiles_stay_within_the_bucket_bounds() {
        let mut histogram = Histogram::default();
        for value in 1..=1000 {
            histogram.record(value);
        }

        assert_eq!(histogram.value_at_quantile(0.50), 500);
        assert_eq!(histogram.value_at_quantile(0.95), 1_000);
        assert_eq!(histogram.max, 1_000);
        assert_eq!(histogram.count, 1_000);
    }

    #[test]
    fn a_large_sample_lands_in_the_overflow_bucket() {
        let mut histogram = Histogram::default();
        histogram.record(u64::MAX);

        assert_eq!(histogram.value_at_quantile(1.0), u64::MAX);
    }
}
