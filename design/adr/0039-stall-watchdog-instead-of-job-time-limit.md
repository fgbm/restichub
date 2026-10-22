# ADR-0039: A stall watchdog instead of a fixed job time limit

- Status: accepted, the grace for a fresh job superseded by ADR-0043, the rule of progress of a backup and a restore refined by ADR-0051
- Date: 2026-10-15
- Requirements: FR-AG-05, FR-AG-10, FR-SC-06
- Refines: ADR-0020, ADR-0021 (job fields), the queue decisions of ADR-0017

## Context

Each queued job carried `timeout_seconds` of 12 hours. The agent enforced it as a hard deadline. The control plane failed a running job as `timed_out` ten minutes after that time. A first backup of hundreds of gigabytes can run longer. A backup over a slow link can also run longer. Both can make progress during the whole run. The job was killed although it made progress. The job detail also showed "Expires" one hour after the start. That value is the acceptance window of the signed job (`jobs.expires_at`), not a limit of the run.

## Decision

### The agent stops a stalled step, not a long one

- A step has no wall-clock limit by default.
- The agent holds a stall watchdog for each running step. A step makes progress when a status line of restic changes `bytes_done`, `files_done` or the current files. It also makes progress when restic prints any other line. A repeated status line with the same numbers is not progress.
- The watchdog stops the step in two stages (FR-AG-10) when it saw no progress for the stall timeout. The result carries the stable code `step_stalled`, and the job result carries the code `stalled`.
- The job field `stall_timeout_seconds` sets the stall timeout. Zero means the default of the step: 60 minutes for `backup`, `restore` and the short steps, and 6 hours for `check` and `prune`, which print nothing for long periods. The verify phase of a `restore` keeps 24 hours from the summary line (ADR-0051).
- A `backup` step with an upload limit raises its stall timeout to at least twice the time to upload one pack at the limited rate. A pack holds 128 MiB at most, the largest size that restic writes. restic stops reading the files while its upload slots are full, and it reports `bytes_done` again only when an upload ends. A shorter window therefore stops a healthy backup on a narrow link. At 50 KiB/s the raised value is 87 minutes, and at 500 KiB/s it is 9 minutes. The agent keeps a longer stall timeout of the job.
- The watchdog is not a schedule. It runs only inside a running step and starts no job (ADR-0002).

### A maximum duration of 48 hours by default

- The job field `timeout_seconds` is the maximum duration. Zero means none. The step field `timeout_seconds` keeps its meaning for one step.
- The agent stops a step that runs longer than the maximum duration, with the codes `step_max_duration` and `max_duration_exceeded`.
- A profile has two settings: the stall timeout (default 60 minutes) and the maximum duration (default 48 hours; zero turns it off). The environment variables `RESTICHUB_JOB_STALL_TIMEOUT_SECS` and `RESTICHUB_JOB_MAX_DURATION_SECS` of the control plane change the defaults of a new profile. The same stall variable on the agent changes the stall timeout of a job that carries none. The control plane compiles them into the signed job. Browse jobs keep their own step timeouts. Maintenance jobs (forget, prune, check) get a maximum duration of 48 hours. The control plane also fails any running job after a hard ceiling (31 days by default, `RESTICHUB_JOB_HARD_CEILING_SECS`), so no job stays `running` forever.

### The control plane does not use a wall clock

- A running job fails as `agent_lost` only when the agent is not connected. The control plane must also not hear from the agent for the lost window (30 minutes, `STALE_LOCK_WAIT`).
- `AgentHello` lists the identifiers of the jobs that the agent runs now or holds a stored result of (`held_job_ids`). The agent sends the stored results right after the hello. When an agent connects, the control plane fails a `running` job that the agent does not list. The agent restarted and lost that job. A job that started a short time ago is not failed, because the acknowledgement and the hello can cross.
- The control plane fails a running job as `timed_out` only when the job has a maximum duration (`timeout_seconds` greater than zero). The grace period stays 10 minutes.
- Rows that exist before the migration keep their stored `timeout_seconds`. The migration sets the old 12 hour value of `queued`, `sent` and `running` rows to NULL. A job that runs now is not killed by the old limit.

### Compatibility

- The capability `job-limits-v1` marks an agent that reads `stall_timeout_seconds` and treats a zero `timeout_seconds` as no limit. An older agent applies its own fixed default of one hour to a zero value. The control plane sends the old 12 hour value to such an agent.

### Display

- The job detail shows the acceptance window only while the job is queued or sent ("Accept until"). A running job shows its limits: none, the stall timeout, or the maximum duration.

## Note of 2026-10-16

ADR-0043 removes the grace rule for a job that started a short time ago. The agent lists a job before it acknowledges it, so the CP fails each running job that the hello omits.

## Note of 2026-10-22

ADR-0051 refines the rule of progress. For a `backup` and a `restore` step, a line on the standard error is not progress. restic prints the retry of an unreachable backend there, and the stall timeout never expired. The verify phase of a `restore` prints nothing. It gets a window of 24 hours from the summary line, or the longer stall timeout of the step. Both values are finite, so a verify phase that a hung target blocks stops the step after that window.

## Consequences

- A backup that makes progress is never stopped by time.
- A job that restic cannot advance (a blocked backend, a hung network share) stops after the stall timeout and leaves a clear reason.
- A user who needs a hard cap sets a shorter maximum duration in the profile. A user who needs a run of more than 48 hours raises it or sets zero.
