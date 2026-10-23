---
title: Prometheus metrics
description: How to scrape the metrics endpoint of the control plane and which metrics it gives.
---

The control plane gives its state to Prometheus on `GET /metrics`. The endpoint uses the text format of Prometheus.

## Access

The endpoint is not public. It needs a credential with the Viewer role at the least. The identifiers of agents, plans and repositories are tenant data, so the control plane never gives them to an anonymous client. A session of a Platform Admin in the platform tenant gets HTTP 403, because the platform tenant holds no jobs. Switch to a tenant first, or use a token of that tenant.

Use an API token with the Viewer role for the scraper. A token belongs to one tenant. The scrape gives the metrics of that tenant and no data of another tenant. To read many tenants, create one token in each tenant and add one scrape job for each token.

1. Create an API token with the role Viewer in the tenant.
2. Put the token value in a file that only the Prometheus user can read.
3. Add a scrape job that reads the file as a bearer token.

```yaml
scrape_configs:
  - job_name: restichub
    scheme: https
    authorization:
      type: Bearer
      credentials_file: /etc/prometheus/restichub-token
    static_configs:
      - targets: ['restichub.example.com']
```

## Metrics

Each sample has the label `tenant` with the tenant identifier. The identifier holds no personal data.

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `restichub_last_success_timestamp_seconds` | gauge | `tenant`, `plan` | Unix time of the last successful backup of the plan. A plan without a success has no sample. |
| `restichub_job_duration_seconds` | histogram | `tenant`, `kind` | Duration of the jobs that ran. The buckets are 1, 5, 15, 30, 60, 120, 300, 600, 1800 and 3600 seconds. |
| `restichub_job_dispatch_delay_seconds` | histogram | `tenant` | Time from the moment a job may run to its first send to the agent. The buckets are 0.05, 0.1, 0.25, 0.5, 1, 2, 3, 5, 10 and 30 seconds. |
| `restichub_jobs_total` | counter | `tenant`, `status` | Jobs that reached the status `succeeded`, `failed`, `expired` or `dead_letter`. |
| `restichub_agent_up` | gauge | `tenant`, `agent` | 1 when the agent holds an open stream, else 0. |
| `restichub_repo_lock_held` | gauge | `tenant`, `repository` | 1 when a waiting or running job of the repository reported a held lock, else 0. |
| `restichub_plan_schedule_failed` | gauge | `tenant`, `plan` | 1 when the last scheduler pass of the plan failed. A plan without a failure has no sample. |
| `restichub_maintenance_schedule_failed` | gauge | `tenant`, `repository`, `task` | 1 when the last scheduler pass of the maintenance schedule failed. A schedule without a failure has no sample. |
| `restichub_agent_connections_refused_total` | counter | `tenant`, `reason` | Agent connections that the agent listener closed at a limit. The reason is `cap` (the connection cap `RESTICHUB_AGENT_MAX_CONNECTIONS`), `anonymous_share` or `anonymous_per_ip` (the limits for connections without a client certificate), `handshakes` or `handshakes_per_ip` (the limits for TLS handshakes in progress). Single mode only. |
| `restichub_agent_accept_errors_total` | counter | `tenant` | Errors of the agent listener when it accepts a connection, for example when the process has no free file descriptor. Single mode only. |

The control plane reuses the aggregates of a tenant for 15 seconds. A scrape interval below 15 seconds therefore shows the same values twice. The value of `restichub_agent_up` is always current.

The agent listener serves all tenants. In multi mode, the endpoint therefore does not show the listener counters. The control plane log shows the refusals and the accept errors in all modes. Each minute that has refusals or accept errors, the log has one warning. The warning gives the count for each reason and the count of accept errors in that minute.

When the pass of a plan or of a maintenance schedule fails, the control plane writes an error to its log. It moves the plan or the schedule to its next fire time, so the next pass does not process it again at once. The failed fire time creates no job. If the control plane cannot compute the schedule, for example when the cron expression is not valid, it disables the plan or the schedule. A successful pass or an edit of the plan or the schedule removes the failure sample.

To alert when a plan or a maintenance schedule fails, use this expression:

```
restichub_plan_schedule_failed == 1 or restichub_maintenance_schedule_failed == 1
```

To alert when a plan does not finish a backup for a day, use this expression:

```
time() - restichub_last_success_timestamp_seconds > 86400
```
