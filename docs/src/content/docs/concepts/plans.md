---
title: Plans
description: Assign a profile to a host with a schedule.
---

A plan assigns one profile to one agent with a schedule. The control plane keeps the schedule and creates the jobs. The agent holds no schedule (ADR-0002).

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

## Schedule

The control plane calculates each fire time from the cron expression and the time zone. A job names the plan and the fire time. The control plane creates one job for each fire time. The pair `(plan, fire time)` is unique, so two control plane instances create one job for one fire time (FR-SC-02, ADR-0010).

The control plane applies the daylight saving rules of the accepted decision:

- A fire time in a daylight saving gap runs once, right after the clock change.
- A fire time in a daylight saving overlap runs once, at the first occurrence.

## Missed runs

A fire time is missed when the control plane does not run a job for it, for example after a downtime. The missed run policy of the plan decides the result:

| Policy | Result |
|---|---|
| Run every missed run | One job for each missed fire time. One pass creates at most 100 jobs; the fire times beyond the limit are dropped. |
| Run the latest missed run | One job, for the latest missed fire time. |
| Skip the missed runs | No job. |

Every scheduled job has an expiry time, 24 hours after its fire time. The control plane moves an overdue job to the `expired` status.

A suspended tenant creates no job. A plan whose agent is revoked creates no job.

## Roles

| Role | Right |
|---|---|
| Admin, Operator | Create, change and delete a plan. |
| Viewer | Read a plan. |

## API

| Method and path | Function |
|---|---|
| `GET /api/v1/plans` | List the plans. The parameters `cursor` and `limit` page the list. |
| `POST /api/v1/plans` | Create a plan. |
| `GET /api/v1/plans/{id}` | Read one plan. |
| `PATCH /api/v1/plans/{id}` | Change a plan. |
| `DELETE /api/v1/plans/{id}` | Delete a plan. |
| `GET /api/v1/plans/{id}/next-runs?count=5` | The next fire times, in UTC. |

A plan of another tenant gives HTTP 404. The platform tenant refuses every plan operation with HTTP 403.
