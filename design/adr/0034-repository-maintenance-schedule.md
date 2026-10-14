# ADR-0034: Repository maintenance schedule and retention isolation

- Status: accepted
- Date: 2026-10-14
- Requirements: FR-RP-08, FR-SC-02, FR-SC-04, FR-PF-01, FR-PF-04, AC-RP-04, AC-SC-03

## Context

[ADR-0010](0010-queue-and-scheduler-on-postgresql.md) gives the queue, the barriers and the plan schedule. A plan creates backup jobs only. The profile holds a retention and check settings, and the compiler already produces the `forget`, `prune` and `check` steps. Nothing creates a job for them.

FR-RP-08 allows only the maintenance agent of a repository to run `forget`, `prune` and `check`. A backup host must never receive them. Several profiles can use one repository, and each profile has its own retention. A `forget` step without a filter applies one retention to every snapshot of the repository, and so a profile can remove the snapshots of another profile.

## Decision

- A repository has two schedules in the table `maintenance_schedules`: the task `retention` and the task `check`. Each row has a cron expression, an IANA time zone, an enabled flag and a next fire time. The defaults are `0 3 * * 0` for the retention (weekly) and `0 4 1 * *` for the check (monthly), both in UTC. The scheduler creates the default rows for a repository that has none.
- The scheduler pass of a tenant claims the due rows with `FOR UPDATE SKIP LOCKED`. A scheduled job names the schedule and the fire time. The unique key `(maintenance_schedule_id, fire_time, attempt)` makes the create idempotent, so two instances create one job for one fire time (ADR-0010).
- The job runs on `repositories.maintenance_agent_id` only. A repository without a maintenance agent, with a revoked one, or that is not initialized gets no maintenance job, and its schedule advances. The job builder refuses a `forget`, `prune` or `check` job for any other agent with the reason `not_maintenance_agent`.
- The retention task is one job of the kind `forget`. It has one `forget` step for each profile with a retention and each agent that runs a plan of that profile. One `prune` step follows. The `prune` step runs after the `forget` steps in the same job, so no second exclusive job waits in the queue. The check task is one job of the kind `check`.
- The task never creates a job for a repository that holds a queued, sent or running exclusive job. The schedule stays due, and a later pass creates the job. A repository without a retention that a plan runs gets no retention job.
- The job is exclusive, so the barriers of ADR-0010 apply. The job starts after the running backups end. The backups queued after it wait for it (AC-SC-03).
- The `check` job reads the `read_data_subset` of the first profile of the repository that sets one. A repository without such a profile gets a check of the structure only.

### Retention isolation

- The job builder adds the tag `restichub-profile-<profile id>` to every backup of a profile. The tag is part of the compiled backup step, not of the stored profile.
- The `forget` step of a profile carries three filters. `--tag` has this tag. `--host` has the host name of the agent. `--path` has each backup path of the agent. The profile tag is the exact key. The host and the paths narrow the step and keep the intent visible in the command line.
- The step uses the grouping of the profile. A profile without a grouping gets `host,paths`.
- A filter in the retention of the profile does not apply to the maintenance job. The isolation filter replaces it.
- No two profiles can conflict. Each snapshot has one profile tag, and only the step of that profile can select it. A profile therefore never saves a conflict at save time, and the control plane needs no merge rule.

### Limits

- A snapshot that a backup made before this decision has no profile tag, and no `forget` step selects it. The operator removes such snapshots by hand.
- A profile with a retention and no plan gives no `forget` step, because the control plane does not know the host of its snapshots.
- A manual run of a task needs the Operator role. The run follows the same barriers as a scheduled job.

## Consequences

- A backup host never receives a maintenance step, and a test proves it (AC-RP-04).
- The retention of one profile cannot delete the snapshots of another profile that shares the repository, the host or the paths.
- The repository dialog shows a warning when no maintenance agent is set.
- Old snapshots without the tag stay until a person removes them.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Filter on host and paths only | Two profiles on one host with overlapping paths share snapshots, so one retention could remove the snapshots of the other profile. |
| One merged retention for the repository | A union of the `keep` rules keeps more than a profile asked for, and an intersection could delete data that a profile wants to keep. |
| Refuse conflicting retention at save time | Without an exact key the control plane cannot tell whether two profiles conflict. |
| A `prune` job behind the `forget` job | The index of the active job allows one exclusive job for each repository, so the second job could not wait in the queue. |
| A maintenance plan row for each repository | A plan binds a profile and an agent, and a repository task has neither. |
