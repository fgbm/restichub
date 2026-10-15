---
title: Jobs
description: How the queue sends jobs to the agents, and how it limits and retries them.
---

A job is one run of a plan or one manual operation. The control plane keeps the jobs in a queue in the database. The queue dispatcher sends each job to an online agent over the open stream.

## Job states

| State | Meaning |
|---|---|
| Waiting (`queued`) | The job is in the queue. The control plane did not send it. |
| Sent (`sent`) | The control plane sent the job. The agent did not confirm it. |
| Running (`running`) | The agent confirmed the job and runs it. |
| Succeeded (`succeeded`) | Every step succeeded. |
| Failed (`failed`) | A step failed, the job was cancelled, or the control plane refused it. The result code and the reason give the detail. |
| Expired (`expired`) | The job waited in the queue longer than its acceptance window. A running job never expires. |
| Gave up (`dead_letter`) | The repository stayed locked after the last retry. |

A job in a final state never changes again.

## Delivery

The dispatcher builds each job when it sends the job for the first time. It compiles the stored profile of the plan again with the facts of the agent. If the profile no longer compiles, the job fails with the reason `compile_failed`. The control plane never sends a stored command.

A job carries the secrets of its own repository only. The control plane reads each secret when it sends the job and writes a `secret_delivered` audit record. The record holds no secret value.

The control plane always sends `--retry-lock` with a restic command.

The control plane signs each job. The agent checks the signature before it runs the job. The control plane keeps the signed job, encrypted with the key of the tenant, until the job reaches a final state.

## Lease and redelivery

When the control plane sends a job, it starts a lease. The default lease is 60 seconds. The smallest lease is 15 seconds. If the agent does not confirm the job before the lease ends, the control plane sends the job again. It sends the same signed bytes and does not build the job again. A change to the profile does not change a job that the control plane sent. Each delivery writes the `secret_delivered` audit records. The job has the same identifier each time. The agent keeps a journal and runs a known job once.

If the agent answers that it is busy, the control plane sends the job again after 10 seconds. A busy answer does not count as a delivery.

If the control plane sent a job five times without a confirmation, the job fails with the reason `not_acknowledged`.

If the agent is offline, the job stays in the queue. The control plane reads no secret for an offline agent.

## Repository barriers

Some operations need a repository for themselves. These are `init`, `check`, `forget`, `prune`, `unlock` and `unlock_all`. The queue calls them exclusive.

- An exclusive job starts only when no other job runs on the repository.
- No other job starts on the repository while an exclusive job runs.
- The queue keeps the order of the jobs. A job that waits for the repository blocks the jobs that the control plane queued after it. A prune does not wait for ever behind a stream of new backups.

Several backups can run on one repository at the same time. The repository limit sets the largest number.

## Limits

The queue applies three limits:

| Limit | Default | Setting |
|---|---|---|
| Jobs on one agent | 2 | `RESTICHUB_QUEUE_AGENT_LIMIT` |
| Backups on one repository | 4 | `RESTICHUB_QUEUE_REPOSITORY_LIMIT` |
| Jobs in one tenant | 100 | `RESTICHUB_QUEUE_GLOBAL_LIMIT` |

The same values can be set in the `[queue]` section of the configuration file, with the keys `agent_limit`, `repository_limit` and `global_limit`. A job that exceeds a limit waits in the queue.

## Time limits

A running job has no short fixed time limit. A first backup of a large disk can take many hours. While restic makes progress, the job runs on until the maximum duration of its profile.

### Acceptance window

A job in the queue has an acceptance window. A manual job has a window of one hour. A scheduled job has a window of 24 hours, and the signature of a job is never valid for more than 23 hours. The agent refuses a job after this time. The job detail shows the end of the window as "Accept until" while the job is waiting or sent. A running job does not expire.

### Stall timeout

The agent watches the progress of each step. restic makes progress when the number of bytes or files changes, when the current files change, or when it prints a line of text. A repeated status line is not progress.

The agent stops a step that makes no progress for the stall timeout. It stops the step in two stages: first the graceful signal, then the forced stop of the process tree after the grace period. The job fails with the reason `stalled`.

| Step | Default stall timeout |
|---|---|
| `backup`, `restore` and the short steps | 60 minutes |
| `check`, `prune` | 6 hours, also inside a backup job |

A profile sets the stall timeout for its jobs. The setting is "Stop after no progress" in the section "Limits" of the profile. The value is from 5 to 10080 minutes. A `check` or `prune` step in the job waits for the longer of this value and 6 hours. A status line that repeats the same content is not progress.

### Maximum duration

A profile sets a maximum duration. The default is 48 hours (2880 minutes). Zero turns the limit off. When the job runs longer, the agent stops it, whatever its progress. The job fails with the reason `max_duration_exceeded`. The value is from 0 to 43200 minutes. The job detail shows the limits of a running job.

### Environment variables

An operator can change the defaults. The values are in seconds.

| Variable | Where | Effect |
|---|---|---|
| `RESTICHUB_JOB_STALL_TIMEOUT_SECS` | control plane | The stall timeout of a new profile. From 300 to 604800. |
| `RESTICHUB_JOB_MAX_DURATION_SECS` | control plane | The maximum duration of a new profile. From 0 to 2592000. Zero means no limit. |
| `RESTICHUB_JOB_STALL_TIMEOUT_SECS` | agent | The stall timeout of a job that carries none, for all step kinds. From 60 to 604800. |
| `RESTICHUB_JOB_HARD_CEILING_SECS` | control plane | The time after which the control plane fails a running job whatever its limits. From 3600. The default is 31 days. |

A value that is not a number, or is out of range, is ignored. A profile that exists keeps its own values.

### Lost jobs

The control plane does not fail a running job because of the time that it ran. It fails the job with the reason `agent_lost` in two cases:

- The agent is offline and the control plane did not hear from it for 30 minutes.
- The agent connects again and does not list the job in its hello message. The agent restarted and lost the job.

A job also fails with the reason `timed_out` when it runs 10 minutes longer than its maximum duration and the agent did not report an end. A job with no maximum duration fails with the same reason at the hard ceiling. Maintenance jobs (forget, prune, check) have a maximum duration of 48 hours. The control plane marks the repository lock as possibly stale in each of these cases.

An agent that is older than this version does not know the stall timeout and does not stop a step that makes no progress. The control plane sends such an agent the maximum duration of the profile, or a fixed limit of 12 hours when the profile has none. The job detail shows only the maximum duration for such an agent.

## Lock retries

Restic exits with code 11 when another operation holds the repository lock. The control plane does not give up the plan. It queues the job again with a delay.

The control plane ends the job that reported the lock and queues a new job with a new identifier. The agent keeps a journal and returns the stored result for a known identifier, so a retry needs its own identifier. The new job has the same plan and fire time, and counts one more attempt. The first delay is 30 seconds. The delay doubles for each retry. The largest delay is one hour. After five lock results, the job moves to the final state `dead_letter` and the control plane writes a `job_dead_letter` audit record. Use this record as the alert.

| Setting | Default |
|---|---|
| `RESTICHUB_QUEUE_LOCK_RETRY_LIMIT` | 5 |
| `RESTICHUB_QUEUE_LOCK_RETRY_BASE_SECS` | 30 |
| `RESTICHUB_QUEUE_LEASE_SECS` | 60 |

## Stale locks

A hard stop of restic can leave a lock in the repository. The control plane marks the lock in two cases. The agent reports a possible stale lock, or the agent is lost while a job runs. A job on a lost agent fails with the reason `agent_lost`. A running job fails with the reason `timed_out` when it stays past its maximum duration and a grace period of 10 minutes. The control plane marks its lock too. The agent stops a stalled step itself. It also stops a step past its maximum duration. It reports `possible_stale_lock` after a forced stop. A sent job that the agent did not confirm before it expired fails with the reason `unacknowledged_expired`, and its lock is marked too.

When the mark is 30 minutes old and no job waits or runs on the repository, the control plane queues an `unlock` job. The job never removes the locks of a running operation. If a later job succeeded on the repository, the control plane queues no unlock.

Only an Admin can remove every lock. The action sends an `unlock_all` job. It needs the repository name as the confirmation. It refuses a repository that has a waiting or running job. The audit log keeps the event `repository_unlock_all`.

## Cancel

An Operator can cancel a job.

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
| `agent_capability_missing` | The agent is too old for the step. It does not know a field that the step needs. |
| `forget_without_tags` | The retention step has no tag filter. The control plane does not send it. |
| `step_missing` | The profile no longer holds the step of the job. |
| `agent_revoked` | The agent is revoked. |
| `plan_missing`, `plan_changed` | The plan is deleted, or it moved to another agent. |
| `profile_missing`, `repository_missing`, `repository_changed` | An object of the job is deleted or changed. |
| `secret_unavailable` | A secret of the repository cannot be read. |
| `signing_failed` | The control plane could not sign the job. |

## Steps, summary and progress

The agent reports each step of a job as a typed result. The control plane stores one row for each step: the status, the exit code, the times and the error code. The agent can send a result twice, so a second write never changes a stored row.

A backup step also carries the restic summary. It holds the new, changed and unchanged files, and the data added to the repository. It also holds the processed files and bytes, the duration and the snapshot identifier. The control plane stores these values as they are. The web UI shows them without a change. A `check`, `forget`, `snapshots` and `stats` step carries its own typed summary.

While a step runs, the agent sends the progress of the step: files, bytes, percent and the file in work. The control plane keeps only the last progress of each job. The job detail shows it for a job that is `sent` or `running`, and removes it when the job ends. The web UI reads the job again every two seconds while the job can still change.

## Job log

The agent sends the log of a job in chunks. It removes every secret before it sends a chunk, and a job log holds at most 1 MiB. The control plane stores one row for each line. Each line has a number, the step, the source (`stdout`, `stderr` or `agent`) and a level. The control plane derives the level from the text of the line: `error`, `warn` or `info`. The agent sends no line time, so the stored time is the time when the control plane stored the line.

The agent sends the tail of the log again after a restart. The agent numbers its chunks for each source. The control plane keeps the number of the chunk with each line. It ignores a chunk that it stored before, in any order of arrival, so a repeated tail never stores a line twice.

Each line counts as its text, one line break and 64 bytes of overhead. The control plane stops at 1 MiB of this count or at 100 000 lines for each job, so empty lines cannot fill the database. After that it keeps no new line and marks the log as truncated. The web UI shows the note. The control plane removes control characters from each line, except the tab. It also removes the Unicode format characters (category Cf, for example U+202E). It cuts a line at 8192 characters.

A finished job takes step and log writes from the agent only during the log retention period after its end.

The log storage is a separate component with a `LogStore` interface. The `database` backend is the default. An `s3` backend is planned for the 1.0 release.

### Log retention

The control plane deletes log lines that are older than the retention period. A sweep runs each hour. The job, its steps and its summary stay: only the log text goes away. The job detail then shows that the log is deleted.

| Setting | Default | Variable |
|---|---|---|
| Days that the control plane keeps the log text | 30 | `RESTICHUB_LOG_RETENTION_DAYS` |

The configuration file holds the same value as `log_retention_days`. The value is the same for every tenant. A value below 1 is raised to 1. A value above 36500 stops the start.

## Roles

| Role | Right |
|---|---|
| Admin, Operator | Run a plan and cancel a job. A plan holds one waiting manual run: a second run gives HTTP 409 `already_queued`. |
| Admin | Run `unlock_all`. |
| Viewer | Read the jobs, their steps and their logs. |

A suspended tenant sends no job. Its agents stay connected.

## API

| Method and path | Function |
|---|---|
| `GET /api/v1/jobs` | List the jobs, newest first. The parameters `plan_id`, `repository_id`, `agent_id` and `status` filter the list. The parameters `cursor` and `limit` page the list. |
| `GET /api/v1/jobs/{id}` | Read one job with its steps, its summary and, for a running job, its progress. |
| `GET /api/v1/jobs/{id}/logs` | Read a page of the log. The parameter `after_seq` is the number of the last line that you hold, and `limit` is the page size (1 to 2000, default 500). Use the `seq` of the last line as the next `after_seq`. |
| `POST /api/v1/jobs/{id}/cancel` | Cancel a job. |
| `POST /api/v1/plans/{id}/run` | Run a plan now. |
| `POST /api/v1/repositories/{id}/unlock-all` | Remove every lock of a repository. The body holds `confirm` with the repository name. |

A job of another tenant gives HTTP 404.
