---
title: Plans
description: Assign a profile to a host and a repository with a schedule.
---

A plan binds one profile to one agent and one repository with a schedule. The control plane keeps the schedule and creates the jobs. The agent holds no schedule.

## Plan fields

| Field | Meaning |
|---|---|
| Name | The name of the plan. It is any text of 1 to 100 characters. It is unique in the tenant, and the letter case does not matter. A new plan gets the name `<host> | <profile>` until you type a name. |
| Profile | The backup definition that the control plane compiles for each job. The field shows the first path and the number of parameters of each profile. The search finds a profile by its name, by a path or by a parameter. |
| Repository | The repository that receives the snapshots of the plan. The field shows the backend kind and the location. For a local repository, it also shows the repository host. The search finds a repository by its name or its location. |
| Host | The agent that runs the plan. |
| Cron expression | Five fields: minute, hour, day of month, month, day of week. |
| Time zone | An IANA name, for example `Europe/Berlin`. |
| Missed runs | The rule for a fire time that the control plane missed, and for the jobs that wait for an offline host. |
| Run on schedule | The switch in the header of the section **Schedule**. When it is off, the scheduler creates no job for the plan, and the plan shows the pill **Disabled** next to its name. **Run now** still starts a job, and the alerts about a missed backup and the recovery point stay silent. |

The profile, the repository and the agent belong to the same tenant. The control plane compiles the stored profile for the operating system of the agent before it writes the plan. A profile without a path for that operating system is refused. The plans of a repository block the delete of the repository.

## Profile parameters

A plan can override a value that the profile declares. The section "Profile parameters" shows each parameter with the value that applies and its source: "plan", "host" or "default". The control plane resolves a value in this order: the plan, then the host, then the default value. A saved plan has a value for each required parameter. See [Profile parameters](/restichub/concepts/profile-parameters/).

## A local repository

The path of a local repository exists on the repository host only. A plan that writes to a local repository therefore runs on the repository host only (ADR-0046). The plan page offers only this host. When you select a local repository, the page selects its host and locks the field "Host". The page does not change the host of a stored plan by itself. If a stored plan runs on another host, the field "Host" shows an error until you select the repository host. The control plane refuses another host with the reason `not_repository_host` on the field "Host". See [A local repository](/restichub/getting-started/add-a-repository/#a-local-repository).

## Schedule

The control plane calculates each fire time from the cron expression and the time zone. A job names the plan and the fire time. The control plane creates one job for each fire time. The pair `(plan, fire time)` is unique, so two control plane instances create one job for one fire time.

The control plane applies the daylight saving rules of the accepted decision:

- A fixed-time fire time in a daylight saving gap runs once, right after the clock change.
- A fixed-time fire time in a daylight saving overlap runs once, at the first occurrence.
- An interval fire time, such as `0 * * * *` or `*/15`, names no wall-clock time. It runs in both halves of an overlap.

### Schedule failure

When the control plane cannot process the schedule of a plan, the plan page shows a message. The message gives the reason, the time of the first failure, and the next step. The plan stays enabled, and its RPO alert stays active. The dashboard counts the plan as at risk.

| Reason | Meaning | Next step |
|---|---|---|
| Database error | The database refused a write. | The control plane keeps the fire time and tries again after a pause. The pause starts at 5 seconds, doubles, and does not exceed 10 minutes. |
| Invalid cron expression | The control plane cannot read the cron expression. | Edit the cron expression. |
| Unknown time zone | The time zone is not in the time zone database. | Edit the time zone. |
| No fire time | The schedule has no next fire time. | Edit the cron expression. |

A successful pass, or an edit of the cron expression, the time zone or the switch "Run on schedule", removes the message. The audit log records the actions `schedule_failed` and `schedule_recovered` for the actor system. A rename of the plan keeps the message.

## Missed runs

A fire time is missed when the control plane does not run a job for it, for example after a downtime. A fire time within a short grace window of the pass is on time. The control plane creates a job for it under every policy. The missed run policy applies to the older fire times only. The policy of the plan decides the result:

| Policy | Result |
|---|---|
| Run every missed run | One job for each older missed fire time. One pass keeps the latest 100 fire times. |
| Run the latest missed run | One job, for the latest older missed fire time. |
| Skip the missed runs | No job for an older missed fire time. |

The policy also decides what happens to the jobs that wait for an offline host:

- With the policy "Run the latest missed run" or "Skip the missed runs", a new scheduled job replaces the older scheduled jobs of the plan. The host did not take these jobs. The replaced jobs get the `expired` status. A host that was offline runs one backup when it returns, for the newest fire time. A job that the queue already sent does not change. A job that you started with Run now does not change.
- With the policy "Run every missed run", the host runs each queued job.

Every scheduled job has an expiry time, 24 hours after its fire time. The control plane creates no job for a fire time that the job would outlive at once, because such a job expires at once. The control plane moves an overdue job to the `expired` status.

A suspended tenant creates no job. A plan whose agent is revoked creates no job.

## Delivery

A scheduled job starts in the `queued` status. The queue sends the job to the online agent. The [Jobs](/restichub/concepts/job-lifecycle/) page describes the queue, the limits and the retries. A plan delete ends the queued jobs of the plan with the `cancelled` result.

## Run now

The action Run now queues one job for the plan at once. The job follows the same rules as a scheduled job. A disabled plan can run by hand. The control plane checks that the profile compiles for the agent before it queues the job.

## Roles

| Role | Right |
|---|---|
| Admin, Operator | Create, change, delete and run a plan. An Operator sets the text parameter values. |
| Admin | Set, change or remove a secret parameter value. Delete a plan that holds a secret value. |
| Viewer | Read a plan and preview a schedule. |

A Viewer sees the names of the parameters of a plan, not the text values.

## API

| Method and path | Function |
|---|---|
| `GET /api/v1/plans` | List the plans. The parameters `cursor` and `limit` page the list. |
| `POST /api/v1/plans` | Create a plan. |
| `GET /api/v1/plans/{id}` | Read one plan. |
| `PATCH /api/v1/plans/{id}` | Change a plan. |
| `DELETE /api/v1/plans/{id}` | Delete a plan. |
| `POST /api/v1/plans/{id}/run` | Queue one job for the plan now. The response holds the job. |
| `GET /api/v1/plans/{id}/next-runs?count=5` | The next fire times, in UTC. |
| `POST /api/v1/plans/preview` | The next fire times of an unsaved schedule, in UTC. The body holds `cron`, `time_zone` and `count`. |

A plan of another tenant gives HTTP 404. The platform tenant refuses every plan operation with HTTP 403.
