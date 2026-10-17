---
title: Plans
description: Assign a profile to a host with a schedule.
---

A plan assigns one profile to one agent with a schedule. The control plane keeps the schedule and creates the jobs. The agent holds no schedule.

## Plan fields

| Field | Meaning |
|---|---|
| Name | The name of the plan. It is unique in the tenant. |
| Profile | The backup definition that the control plane compiles for each job. |
| Host | The agent that runs the plan. |
| Cron expression | Five fields: minute, hour, day of month, month, day of week. |
| Time zone | An IANA name, for example `Europe/Berlin`. |
| Missed runs | The rule for a fire time that the control plane missed. |
| Enabled | A disabled plan creates no job. |

The profile and the agent belong to the same tenant. The control plane compiles the stored profile for the operating system of the agent before it writes the plan. A profile without a path for that operating system is refused.

## A local repository

The path of a local repository exists on the repository host only. A plan of a profile that writes to a local repository therefore runs on the repository host only. The plan page offers only this host, and it selects the host when you select the profile. The control plane refuses another host with the reason `not_repository_host` on the field "Host". It also refuses a profile change to a local repository while a plan of the profile runs on another host. See [A local repository](/restichub/guides/add-a-repository/#a-local-repository).

## Schedule

The control plane calculates each fire time from the cron expression and the time zone. A job names the plan and the fire time. The control plane creates one job for each fire time. The pair `(plan, fire time)` is unique, so two control plane instances create one job for one fire time.

The control plane applies the daylight saving rules of the accepted decision:

- A fixed-time fire time in a daylight saving gap runs once, right after the clock change.
- A fixed-time fire time in a daylight saving overlap runs once, at the first occurrence.
- An interval fire time, such as `0 * * * *` or `*/15`, names no wall-clock time. It runs in both halves of an overlap.

## Missed runs

A fire time is missed when the control plane does not run a job for it, for example after a downtime. A fire time within a short grace window of the pass is on time, and it always runs. The missed run policy applies to the older fire times only. The policy of the plan decides the result:

| Policy | Result |
|---|---|
| Run every missed run | One job for each older missed fire time. One pass keeps the latest 100 fire times. |
| Run the latest missed run | One job, for the latest older missed fire time. |
| Skip the missed runs | No job for an older missed fire time. |

Every scheduled job has an expiry time, 24 hours after its fire time. The control plane creates no job for a fire time that the job would outlive at once, because such a job expires at once. The control plane moves an overdue job to the `expired` status.

A suspended tenant creates no job. A plan whose agent is revoked creates no job.

## Delivery

A scheduled job starts in the `queued` status. The queue sends the job to the online agent. The [Jobs](/restichub/concepts/job-lifecycle/) page describes the queue, the limits and the retries. A plan delete ends the queued jobs of the plan with the `cancelled` result.

## Run now

The action Run now queues one job for the plan at once. The job follows the same rules as a scheduled job. A disabled plan can run by hand. The control plane checks that the profile compiles for the agent before it queues the job.

## Roles

| Role | Right |
|---|---|
| Admin, Operator | Create, change, delete and run a plan. |
| Viewer | Read a plan and preview a schedule. |

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
