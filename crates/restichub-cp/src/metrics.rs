//! The Prometheus endpoint (FR-MN-05, AC-MN-03).
//!
//! `GET /metrics` needs a credential with the Viewer role. The recommended scraper credential is an
//! API token with the Viewer role: it reads the metrics of its own tenant and nothing else, and it
//! cannot start a job. The endpoint is never public, because the identifiers of agents, plans and
//! repositories are tenant data.
//!
//! Each sample carries the label `tenant` with the tenant identifier. The identifier holds no
//! personal data, unlike a tenant name.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use restichub_db::metrics::{self as metrics_repo, DURATION_BUCKETS, MetricsSnapshot};
use restichub_db::{AgentId, TenantId};

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::server::AppState;

const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Renders the metrics of the tenant of the credential in the Prometheus text format.
#[utoipa::path(
    get,
    path = "/metrics",
    summary = "Prometheus metrics of the tenant.",
    description = "The text format of Prometheus. The credential needs the Viewer role; use an API token with the Viewer role for the scraper. Each sample has the label `tenant`.",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The metrics.", content_type = "text/plain", body = String),
        crate::error::ProtectedResponses,
    )
)]
pub async fn metrics(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Response, ApiError> {
    context.authorize(Operation::MetricsRead)?;
    let snapshot = context
        .tenant_tx(&state, async |tx| Ok(metrics_repo::snapshot(tx).await?))
        .await?;
    let registry = state.agent_registry();
    let body = render(context.tenant_id, &snapshot, |agent| {
        registry.is_online(agent)
    });
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE));
    Ok(response)
}

/// Writes the exposition text for one tenant. `online` tells whether an agent holds a stream.
pub fn render(
    tenant: TenantId,
    snapshot: &MetricsSnapshot,
    online: impl Fn(AgentId) -> bool,
) -> String {
    let tenant = tenant.to_string();
    let mut out = String::new();

    family(
        &mut out,
        "restichub_last_success_timestamp_seconds",
        "gauge",
        "Unix time of the last successful backup of a plan.",
    );
    for (plan, time) in &snapshot.last_success {
        let _ = writeln!(
            out,
            "restichub_last_success_timestamp_seconds{{tenant=\"{tenant}\",plan=\"{plan}\"}} {}",
            time.unix_timestamp()
        );
    }

    family(
        &mut out,
        "restichub_job_duration_seconds",
        "histogram",
        "Duration of the jobs that ran, by kind.",
    );
    for (kind, histogram) in &snapshot.durations {
        for (bound, count) in DURATION_BUCKETS.iter().zip(&histogram.buckets) {
            let _ = writeln!(
                out,
                "restichub_job_duration_seconds_bucket{{tenant=\"{tenant}\",kind=\"{kind}\",le=\"{bound}\"}} {count}"
            );
        }
        let _ = writeln!(
            out,
            "restichub_job_duration_seconds_bucket{{tenant=\"{tenant}\",kind=\"{kind}\",le=\"+Inf\"}} {}",
            histogram.count
        );
        let _ = writeln!(
            out,
            "restichub_job_duration_seconds_sum{{tenant=\"{tenant}\",kind=\"{kind}\"}} {}",
            histogram.sum_seconds
        );
        let _ = writeln!(
            out,
            "restichub_job_duration_seconds_count{{tenant=\"{tenant}\",kind=\"{kind}\"}} {}",
            histogram.count
        );
    }

    family(
        &mut out,
        "restichub_jobs_total",
        "counter",
        "Jobs that reached a final status.",
    );
    for (status, count) in &snapshot.jobs_total {
        let _ = writeln!(
            out,
            "restichub_jobs_total{{tenant=\"{tenant}\",status=\"{}\"}} {count}",
            status.as_str()
        );
    }

    family(
        &mut out,
        "restichub_agent_up",
        "gauge",
        "1 when the agent holds an open stream, else 0.",
    );
    for agent in &snapshot.agents {
        let _ = writeln!(
            out,
            "restichub_agent_up{{tenant=\"{tenant}\",agent=\"{agent}\"}} {}",
            u8::from(online(*agent))
        );
    }

    family(
        &mut out,
        "restichub_repo_lock_held",
        "gauge",
        "1 when a job of the repository reported a held lock and has not ended, else 0.",
    );
    for (repository, held) in &snapshot.repo_lock_held {
        let _ = writeln!(
            out,
            "restichub_repo_lock_held{{tenant=\"{tenant}\",repository=\"{repository}\"}} {}",
            u8::from(*held)
        );
    }
    out
}

fn family(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}
