# ADR-0039: A stall watchdog instead of a fixed job time limit

- Status: accepted
- Date: 2026-10-15
- Requirements: FR-AG-05, FR-AG-10, FR-SC-06
- Refines: ADR-0020, ADR-0021 (job fields), the queue decisions of ADR-0017

## Context

Each queued job carried `timeout_seconds` of 12 hours. The agent enforced it as a hard deadline. The control plane failed a running job as `timed_out` ten minutes after that time. A first backup of hundreds of gigabytes, or a backup over a slow link, can run longer, and it can make progress during the whole run. The job was killed although it made progress. The job detail also showed "Expires" one hour after the start. That value is the acceptance window of the signed job (`jobs.expires_at`), not a limit of the run.

## Decision

### The agent stops a stalled step, not a long one

- A step has no wall-clock limit by default.
- The agent holds a stall watchdog for each running step. The step makes progress when restic prints a status line whose `bytes_done`, `files_done` or current files changed since the last status line, or when restic prints any other line. A repeated status line with the same numbers is not progress.
- The watchdog stops the step in two stages (FR-AG-10) when it saw no progress for the stall timeout. The result carries the stable code `step_stalled`, and the job result carries the code `stalled`.
- The job field `stall_timeout_seconds` sets the stall timeout. Zero means the default of the step: 60 minutes for `backup`, `restore` and the short steps, and 6 hours for `check` and `prune`, which print nothing for long periods.
- The watchdog is not a schedule. It runs only inside a running step and starts no job (ADR-0002).

### An optional maximum duration

- The job field `timeout_seconds` is the optional maximum duration. Zero means none. The step field `timeout_seconds` keeps its meaning for one step.
- The agent stops a step that runs longer than the maximum duration, with the codes `step_max_duration` and `max_duration_exceeded`.
- A profile has two settings: the stall timeout (default 60 minutes) and the maximum duration (default off). The control plane compiles them into the signed job. Jobs that the control plane creates for itself (browse, maintenance) keep their own values.

### The control plane does not use a wall clock

- A running job fails as `agent_lost` only when the agent is not connected and the control plane did not hear from the agent for the lost window (30 minutes, `STALE_LOCK_WAIT`), as before.
- `AgentHello` lists the identifiers of the jobs that the agent runs now (`running_job_ids`). When an agent connects, the control plane fails a `running` job of that agent that the agent does not list, because the agent restarted and lost the job. A job that started less than a short grace ago is not failed, because the acknowledgement and the hello can cross.
- The control plane fails a running job as `timed_out` only when the job carries a maximum duration (`timeout_seconds` greater than zero). The grace period stays 10 minutes.
- Rows that exist before the migration keep their stored `timeout_seconds`. The migration sets the value of `running` and `sent` rows that carry the old 12 hour value to NULL, so a job that runs now is not killed by the old limit.

### Compatibility

- The capability `job-limits-v1` marks an agent that reads `stall_timeout_seconds` and treats a zero `timeout_seconds` as no limit. An older agent applies its own fixed default of one hour to a zero value. The control plane sends the old 12 hour value to such an agent.

### Display

- The job detail shows the acceptance window only while the job is queued or sent ("Accept until"). A running job shows its limits: none, the stall timeout, or the maximum duration.

## Consequences

- A backup that makes progress is never stopped by time.
- A job that restic cannot advance (a blocked backend, a hung network share) stops after the stall timeout and leaves a clear reason.
- A user who needs a hard cap sets the maximum duration of the profile.
