//! The evaluation of the alert rules (FR-MN-03, AC-MN-01, AC-MN-02, ADR-0036).
//!
//! One pass reads the state of a tenant, builds the list of the conditions that hold, and compares
//! it with the open alerts. A new condition opens an alert; a cleared condition resolves it. Each
//! change writes its deliveries in the same transaction.

use std::collections::HashSet;

use restichub_db::alerts::{
    self, AlertKind, AlertParams, AlertRule, DeliveryEvent, NewAlert, purge_finished_deliveries,
    purge_resolved,
};
use restichub_db::dashboard::{self, AgentFacts, LatestJob, RepositoryLockFacts, RpoPlan};
use restichub_db::{
    AgentId, AgentStatus, AlertId, Database, DbError, JobId, JobStatus, PlanId, RepositoryId,
    TenantId,
};
use time::{Duration, OffsetDateTime};

use super::message::{MAX_TEXT_CHARS, scrub};
use super::rpo::{self, RpoState};
use crate::queue::AgentView;
use time::format_description::well_known::Rfc3339;

/// A job failure older than this at the first look raises no alert.
const NEW_JOB_WINDOW: Duration = Duration::hours(24);

/// Window of the finished jobs that one pass reads. An open alert whose job is older resolves silently.
const JOB_LOOKBACK: Duration = Duration::days(30);

/// How long a job alert of a one-time kind stays open without a later job of the same kind
/// (ADR-0050).
const ONE_TIME_JOB_ALERT_TTL: Duration = Duration::days(7);

/// The alert kinds that follow the latest finished job of a subject.
const JOB_ALERT_KINDS: [AlertKind; 3] = [
    AlertKind::JobFailed,
    AlertKind::JobDeadLetter,
    AlertKind::JobWarnings,
];

/// How long the control plane keeps a resolved alert.
const RESOLVED_RETENTION: Duration = Duration::days(90);

/// How long the control plane keeps a finished delivery.
const DELIVERY_RETENTION: Duration = Duration::days(30);

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub opened: usize,
    pub resolved: usize,
    /// True when another instance held the lock of the tenant.
    pub skipped: bool,
}

#[derive(Debug, Clone)]
struct Candidate {
    kind: AlertKind,
    key: String,
    agent_id: Option<AgentId>,
    plan_id: Option<PlanId>,
    repository_id: Option<RepositoryId>,
    job_id: Option<JobId>,
    agent_hostname: Option<String>,
    plan_name: Option<String>,
    repository_name: Option<String>,
    params: AlertParams,
    detail: String,
}

#[derive(Debug, Default)]
struct Findings {
    active: Vec<Candidate>,
    /// The conditions that are known to hold no more. A recovery notification follows.
    cleared: HashSet<(AlertKind, String)>,
}

/// Runs one evaluation of the tenant. A second instance that runs the same pass at the same time
/// skips it.
///
/// The view says how long this instance read the agents of the tenant without a break. A database
/// outage hides the contact of every agent, so the pass judges no absence before the agent had the
/// time to connect again (AC-ID-06, AC-PR-07). In that time it neither opens nor closes an alert of
/// an absent host: the host may be on its way back, and a close would send a recovery message for a
/// fault that never went away.
pub async fn evaluate_tenant(
    db: &Database,
    tenant: TenantId,
    now: OffsetDateTime,
    view: &AgentView,
) -> Result<Report, DbError> {
    let judges_absence = view.judges_absence(tenant, now);
    let mut tx = db.begin_tenant(tenant).await?;
    tx.force_custom_plan().await?;
    if !alerts::try_lock_tenant(&mut tx).await? {
        drop(tx);
        return Ok(Report {
            skipped: true,
            ..Report::default()
        });
    }
    let rules = alerts::list_rules(&mut tx).await?;
    let mut findings = Findings::default();
    let open = alerts::open_alerts(&mut tx).await?;
    let open_keys: HashSet<(AlertKind, &str)> = open
        .iter()
        .map(|alert| (alert.kind, alert.dedupe_key.as_str()))
        .collect();

    if enabled(&rules, AlertKind::RpoViolated) {
        let plans = dashboard::rpo_plans(&mut tx).await?;
        rpo_findings(&plans, now, &mut findings);
    }
    if judges_absence && let Some(minutes) = threshold(&rules, AlertKind::AgentOffline) {
        let agents = dashboard::agent_facts(&mut tx).await?;
        agent_findings(&agents, minutes, now, &mut findings);
    }
    if let Some(minutes) = threshold(&rules, AlertKind::StaleLock) {
        let repositories = dashboard::repository_locks(&mut tx).await?;
        lock_findings(&repositories, minutes, now, &mut findings);
    }
    if JOB_ALERT_KINDS.iter().any(|kind| enabled(&rules, *kind)) {
        let jobs = dashboard::latest_jobs(&mut tx, now - JOB_LOOKBACK).await?;
        job_findings(&jobs, &rules, now, &open_keys, &mut findings);
    }

    let mut report = Report::default();
    let active_keys: HashSet<(AlertKind, &str)> = findings
        .active
        .iter()
        .map(|candidate| (candidate.kind, candidate.key.as_str()))
        .collect();
    for alert in &open {
        if active_keys.contains(&(alert.kind, alert.dedupe_key.as_str())) {
            continue;
        }
        // An open absence alert waits for the grace too: its agent may be on its way back.
        if alert.kind == AlertKind::AgentOffline && !judges_absence {
            continue;
        }
        if alerts::resolve(&mut tx, alert.id, now).await? {
            report.resolved += 1;
            if findings
                .cleared
                .contains(&(alert.kind, alert.dedupe_key.clone()))
            {
                alerts::enqueue(&mut tx, alert.id, DeliveryEvent::Resolved, now).await?;
            }
        }
    }
    for candidate in &findings.active {
        if open_keys.contains(&(candidate.kind, candidate.key.as_str())) {
            continue;
        }
        let id = AlertId::new();
        let inserted = alerts::insert_open(
            &mut tx,
            &NewAlert {
                id,
                kind: candidate.kind,
                dedupe_key: &candidate.key,
                agent_id: candidate.agent_id,
                plan_id: candidate.plan_id,
                repository_id: candidate.repository_id,
                job_id: candidate.job_id,
                agent_hostname: candidate.agent_hostname.as_deref(),
                plan_name: candidate.plan_name.as_deref(),
                repository_name: candidate.repository_name.as_deref(),
                params: &candidate.params,
                detail: &candidate.detail,
            },
            now,
        )
        .await?;
        // A conflict means that an alert of the same kind and key is open already.
        if inserted {
            alerts::enqueue(&mut tx, id, DeliveryEvent::Opened, now).await?;
            report.opened += 1;
        }
    }

    purge_resolved(&mut tx, now - RESOLVED_RETENTION).await?;
    purge_finished_deliveries(&mut tx, now - DELIVERY_RETENTION).await?;
    tx.commit().await?;
    Ok(report)
}

fn rule(rules: &[AlertRule], kind: AlertKind) -> Option<&AlertRule> {
    rules.iter().find(|rule| rule.kind == kind)
}

fn enabled(rules: &[AlertRule], kind: AlertKind) -> bool {
    rule(rules, kind).is_some_and(|rule| rule.enabled)
}

/// Minutes of an enabled rule. The default fills a missing value.
fn threshold(rules: &[AlertRule], kind: AlertKind) -> Option<i64> {
    let rule = rule(rules, kind).filter(|rule| rule.enabled)?;
    Some(i64::from(rule.threshold_minutes.unwrap_or_else(|| {
        AlertRule::default_for(kind).threshold_minutes.unwrap_or(30)
    })))
}

fn rpo_findings(plans: &[RpoPlan], now: OffsetDateTime, findings: &mut Findings) {
    for plan in plans {
        // A disabled plan and a plan of a revoked agent raise no alert (ADR-0036).
        if !plan.enabled || plan.agent_status != AgentStatus::Active {
            continue;
        }
        let key = format!("plan:{}", plan.id);
        let status = rpo::evaluate(plan, now);
        if status.state != RpoState::Violated {
            findings.cleared.insert((AlertKind::RpoViolated, key));
            continue;
        }
        let params = AlertParams {
            elapsed_seconds: Some(status.age.whole_seconds()),
            rpo_seconds: Some(status.rpo.whole_seconds()),
            last_success_at: status
                .has_success
                .then(|| status.reference.format(&Rfc3339).unwrap_or_default()),
            ..AlertParams::default()
        };
        findings.active.push(Candidate {
            kind: AlertKind::RpoViolated,
            key,
            agent_id: Some(plan.agent_id),
            plan_id: Some(plan.id),
            repository_id: Some(plan.repository_id),
            job_id: plan.last_success_job_id,
            agent_hostname: Some(plan.agent_hostname.clone()),
            plan_name: Some(plan.name.clone()),
            repository_name: Some(plan.repository_name.clone()),
            params,
            detail: String::new(),
        });
    }
}

fn agent_findings(
    agents: &[AgentFacts],
    minutes: i64,
    now: OffsetDateTime,
    findings: &mut Findings,
) {
    let limit = Duration::minutes(minutes);
    for agent in agents
        .iter()
        .filter(|agent| agent.status == AgentStatus::Active)
    {
        let key = format!("agent:{}", agent.id);
        let silent = now - agent.last_contact();
        if silent <= limit {
            findings.cleared.insert((AlertKind::AgentOffline, key));
            continue;
        }
        findings.active.push(Candidate {
            kind: AlertKind::AgentOffline,
            key,
            agent_id: Some(agent.id),
            plan_id: None,
            repository_id: None,
            job_id: None,
            agent_hostname: Some(agent.hostname.clone()),
            plan_name: None,
            repository_name: None,
            params: AlertParams {
                elapsed_seconds: Some(silent.whole_seconds()),
                threshold_minutes: Some(minutes),
                ..AlertParams::default()
            },
            detail: String::new(),
        });
    }
}

fn lock_findings(
    repositories: &[RepositoryLockFacts],
    minutes: i64,
    now: OffsetDateTime,
    findings: &mut Findings,
) {
    let limit = Duration::minutes(minutes);
    for repository in repositories {
        let key = format!("repo:{}", repository.id);
        let stale = repository.stale_since.filter(|since| now - *since > limit);
        let Some(since) = stale else {
            findings.cleared.insert((AlertKind::StaleLock, key));
            continue;
        };
        findings.active.push(Candidate {
            kind: AlertKind::StaleLock,
            key,
            agent_id: repository.stale_agent_id,
            plan_id: None,
            repository_id: Some(repository.id),
            job_id: repository.stale_job_id,
            agent_hostname: repository.stale_agent_hostname.clone(),
            plan_name: None,
            repository_name: Some(repository.name.clone()),
            params: AlertParams {
                elapsed_seconds: Some((now - since).whole_seconds()),
                threshold_minutes: Some(minutes),
                ..AlertParams::default()
            },
            detail: String::new(),
        });
    }
}

/// The alert kind that a finished job raises, or `None` for a healthy job.
fn job_kind(job: &LatestJob) -> Option<AlertKind> {
    match job.status {
        JobStatus::Failed => Some(AlertKind::JobFailed),
        JobStatus::DeadLetter => Some(AlertKind::JobDeadLetter),
        JobStatus::Succeeded if job.result_code.as_deref() == Some("succeeded_with_warnings") => {
            Some(AlertKind::JobWarnings)
        }
        _ => None,
    }
}

fn job_findings(
    jobs: &[LatestJob],
    rules: &[AlertRule],
    now: OffsetDateTime,
    open: &HashSet<(AlertKind, &str)>,
    findings: &mut Findings,
) {
    for job in jobs {
        let Some(kind) = job_kind(job) else {
            for kind in JOB_ALERT_KINDS {
                findings.cleared.insert((kind, job.subject.clone()));
            }
            continue;
        };
        if !enabled(rules, kind) {
            continue;
        }
        // A one-time job of no plan does not run again, so its alert closes at the end of the
        // window instead of waiting for a success that does not come (ADR-0050). The close sends
        // the recovery notification like any other cleared condition.
        if job.plan_id.is_none()
            && job.kind.is_one_time()
            && now - job.finished_at > ONE_TIME_JOB_ALERT_TTL
        {
            for kind in JOB_ALERT_KINDS {
                findings.cleared.insert((kind, job.subject.clone()));
            }
            continue;
        }
        // An old failure raises no new alert, but an alert that is open stays open.
        let known = open.contains(&(kind, job.subject.as_str()));
        if !known && now - job.finished_at > NEW_JOB_WINDOW {
            continue;
        }
        findings.active.push(Candidate {
            kind,
            key: job.subject.clone(),
            agent_id: Some(job.agent_id),
            plan_id: job.plan_id,
            repository_id: job.repository_id,
            job_id: Some(job.id),
            agent_hostname: Some(job.agent_hostname.clone()),
            plan_name: job.plan_name.clone(),
            repository_name: job.repository_name.clone(),
            params: AlertParams {
                job_kind: Some(job.kind),
                error_code: (kind == AlertKind::JobFailed).then(|| {
                    job.result_code
                        .clone()
                        .unwrap_or_else(|| "failed".to_owned())
                }),
                ..AlertParams::default()
            },
            detail: job_detail(job, kind),
        });
    }
}

/// The scrubbed error text of a failed job. Other kinds carry none.
fn job_detail(job: &LatestJob, kind: AlertKind) -> String {
    if kind != AlertKind::JobFailed {
        return String::new();
    }
    job.result_message
        .as_deref()
        .map(|message| scrub(message, MAX_TEXT_CHARS))
        .unwrap_or_default()
}
