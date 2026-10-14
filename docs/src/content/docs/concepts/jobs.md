---
title: Jobs
description: How the queue sends jobs to the agents, and how it limits and retries them.
---

A job is one run of a plan or one manual operation. The control plane keeps the jobs in a queue in the database. The queue dispatcher sends each job to an online agent over the open stream (FR-SC-03, ADR-0010).

## Job states

| State | Meaning |
|---|---|
| Waiting (`queued`) | The job is in the queue. The control plane did not send it. |
| Sent (`sent`) | The control plane sent the job. The agent did not confirm it. |
| Running (`running`) | The agent confirmed the job and runs it. |
| Succeeded (`succeeded`) | Every step succeeded. |
| Failed (`failed`) | A step failed, the job was cancelled, or the control plane refused it. The result code and the reason give the detail. |
| Expired (`expired`) | The job waited in the queue longer than its queue lifetime. A running job never expires. |
| Gave up (`dead_letter`) | The repository stayed locked after the last retry. |

A job in a final state never changes again.

## Delivery

The dispatcher builds each job when it sends the job for the first time. It compiles the stored profile of the plan again with the facts of the agent. If the profile no longer compiles, the job fails with the reason `compile_failed`. The control plane never sends a stored command.

A job carries the secrets of its own repository only (AC-PR-03). The control plane reads each secret when it sends the job and writes a `secret_delivered` audit record. The record holds no secret value (AC-RP-03).

The control plane always sends `--retry-lock` with a restic command (FR-SC-05).

The control plane signs each job. The agent checks the signature before it runs the job (FR-PR-03). The control plane keeps the signed job, encrypted with the key of the tenant, until the job reaches a final state.

## Lease and redelivery

When the control plane sends a job, it starts a lease. The default lease is 60 seconds. The smallest lease is 15 seconds. If the agent does not confirm the job before the lease ends, the control plane sends the job again (FR-PR-06, AC-PR-04). It sends the same signed bytes and does not build the job again. A change to the profile does not change a job that the control plane sent. Each delivery writes the `secret_delivered` audit records. The job has the same identifier each time. The agent keeps a journal and runs a known job once.

If the agent answers that it is busy, the control plane sends the job again after 10 seconds. A busy answer does not count as a delivery.

If the control plane sent a job five times without a confirmation, the job fails with the reason `not_acknowledged`.

If the agent is offline, the job stays in the queue. The control plane reads no secret for an offline agent.

## Repository barriers

Some operations need a repository for themselves. These are `init`, `check`, `forget`, `prune`, `unlock` and `unlock_all`. The queue calls them exclusive (FR-SC-04).

- An exclusive job starts only when no other job runs on the repository.
- No other job starts on the repository while an exclusive job runs.
- The queue keeps the order of the jobs. A job that waits for the repository blocks the jobs that the control plane queued after it. A prune does not wait for ever behind a stream of new backups.

Several backups can run on one repository at the same time. The repository limit sets the largest number.

## Limits

The queue applies three limits (FR-SC-08, AC-SC-08):

| Limit | Default | Setting |
|---|---|---|
| Jobs on one agent | 2 | `RESTICHUB_QUEUE_AGENT_LIMIT` |
| Backups on one repository | 4 | `RESTICHUB_QUEUE_REPOSITORY_LIMIT` |
| Jobs in one tenant | 100 | `RESTICHUB_QUEUE_GLOBAL_LIMIT` |

The same values can be set in the `[queue]` section of the configuration file, with the keys `agent_limit`, `repository_limit` and `global_limit`. A job that exceeds a limit waits in the queue.

## Lock retries

Restic exits with code 11 when another operation holds the repository lock. The control plane does not give up the plan. It queues the job again with a delay (FR-SC-05, AC-SC-05).

The control plane ends the job that reported the lock and queues a new job with a new identifier. The agent keeps a journal and returns the stored result for a known identifier, so a retry needs its own identifier. The new job has the same plan and fire time, and counts one more attempt. The first delay is 30 seconds. The delay doubles for each retry. The largest delay is one hour. After five lock results, the job moves to the final state `dead_letter` and the control plane writes a `job_dead_letter` audit record. Use this record as the alert.

| Setting | Default |
|---|---|
| `RESTICHUB_QUEUE_LOCK_RETRY_LIMIT` | 5 |
| `RESTICHUB_QUEUE_LOCK_RETRY_BASE_SECS` | 30 |
| `RESTICHUB_QUEUE_LEASE_SECS` | 60 |

## Stale locks

A hard stop of restic can leave a lock in the repository. The control plane marks the lock when the agent reports a possible stale lock, or when the agent is lost while a job runs. A job on a lost agent fails with the reason `agent_lost`.

When the mark is 30 minutes old and no job waits or runs on the repository, the control plane queues an `unlock` job (FR-SC-06, AC-SC-06). The job never removes the locks of a running operation. If a later job succeeded on the repository, the control plane queues no unlock.

Only an Admin can remove every lock. The action sends an `unlock_all` job. It needs the repository name as the confirmation. It refuses a repository that has a waiting or running job. The audit log keeps the event `repository_unlock_all`.

## Cancel

An Operator can cancel a job (FR-SC-09).

- A waiting job ends at once with the result `cancelled`.
- For a sent or running job, the control plane marks the job and sends a cancel message to the agent. The queue never sends a marked job. If the agent confirms a marked job late, the control plane sends the cancel message again. A marked job that the agent never confirmed fails with the result `cancelled` when its lease ends. If the agent of a running job is offline, the control plane answers with HTTP 409 `agent_offline`.
- Only an Admin can cancel an `unlock_all` job.
- A job in a final state cannot be cancelled.

## Refused jobs

If the control plane cannot build a job, the job fails with a reason code:

| Reason | Meaning |
|---|---|
| `compile_failed` | The profile no longer compiles for the agent. |
| `agent_facts_invalid` | The facts of the agent are not usable. |
| `step_missing` | The profile no longer holds the step of the job. |
| `agent_revoked` | The agent is revoked. |
| `plan_missing`, `plan_changed` | The plan is deleted, or it moved to another agent. |
| `profile_missing`, `repository_missing`, `repository_changed` | An object of the job is deleted or changed. |
| `secret_unavailable` | A secret of the repository cannot be read. |
| `signing_failed` | The control plane could not sign the job. |

## Roles

| Role | Right |
|---|---|
| Admin, Operator | Run a plan and cancel a job. A plan holds one waiting manual run: a second run gives HTTP 409 `already_queued`. |
| Admin | Run `unlock_all`. |
| Viewer | Read the jobs. |

A suspended tenant sends no job. Its agents stay connected (AC-TN-03).

## API

| Method and path | Function |
|---|---|
| `GET /api/v1/jobs` | List the jobs, newest first. The parameters `plan_id`, `repository_id`, `agent_id` and `status` filter the list. The parameters `cursor` and `limit` page the list. |
| `GET /api/v1/jobs/{id}` | Read one job. |
| `POST /api/v1/jobs/{id}/cancel` | Cancel a job. |
| `POST /api/v1/plans/{id}/run` | Run a plan now. |
| `POST /api/v1/repositories/{id}/unlock-all` | Remove every lock of a repository. The body holds `confirm` with the repository name. |

A job of another tenant gives HTTP 404.
