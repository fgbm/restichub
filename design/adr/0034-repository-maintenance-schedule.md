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
- The retention task is one job of the kind `forget`. It has one `forget` step for each profile with a retention that a plan of an active agent runs. One `prune` step follows. The `prune` step runs after the `forget` steps in the same job, so no second exclusive job waits in the queue. The check task is one job of the kind `check`.
- The task never creates a job for a repository that holds a queued, sent or running exclusive job. The schedule stays due, and a later pass creates the job. A repository without a retention that a plan runs gets no retention job.
- The job is exclusive, so the barriers of ADR-0010 apply. The job starts after the running backups end. The backups queued after it wait for it (AC-SC-03).
- The `check` job reads the largest `read_data_subset` among the profiles of the repository. A tie goes to the oldest profile. A repository without such a profile gets a check of the structure only.

### Retention isolation

- The job builder adds the tag `restichub-profile-<profile id>` to every backup of a profile. The tag is part of the compiled backup step, not of the stored profile.
- The tag prefix `restichub-` is reserved. The API refuses a profile tag, a `retention.keep_tags` entry and a `retention.filter.tags` entry that start with it. The reason is `reserved`.
- A profile gives one `forget` step. The step filters on the profile tag only. The control plane does not filter on the host name of the agent, because an Admin can edit this name. It does not filter on the paths of the backup, because the tag is the exact key.
- The step uses the grouping of the profile. A profile without a grouping gets `host,paths`, so each host and each path set keeps its own policy.
- The filter in the retention of the profile joins the isolation filter as an intersection. `filter.tags` must all be present together with the profile tag: the step has one `--tag=restichub-profile-<profile id>,<tag>,...` value, which restic reads as AND. `filter.paths` narrows the step. `filter.hosts` narrows the step with `--host`, and the control plane skips a plan agent whose host name is not in the list. The host name of the agent compares with the host name that restic records in the snapshot.
- The `filter.tags` setting always means AND with the profile tag. The `forget` step that follows a backup in the compiled plan uses the same filter, so one setting has one meaning. The compiler writes the profile tag and the filter tags to `SnapshotsFilter.all_tags`.
- The agent and the shared validator refuse a `forget` step whose filter has neither `tags` nor `all_tags` (`malformed`), so a step without the tag never reaches the repository.
- A profile can run on several agents. The maintenance step always groups by `host`: the control plane adds `host` to a grouping that lacks it. The retention of one host therefore never counts the snapshots of another host, and each host always keeps its own snapshots.
- The API refuses a template in `retention.filter.hosts`. The maintenance step covers every agent of the plans, so a host name that depends on one agent has no single meaning. A stored profile that has such a template expands it with the facts of the first agent.
- A template that expands to the reserved prefix (case-insensitive) fails to compile. The check runs after the expansion, so it also covers a profile that was saved before the check.
- `SnapshotsFilter.all_tags` is a new field. An agent that does not know it drops it and would run `forget` without the tag. The agent therefore announces the capability `forget-all-tags` in `AgentHello.capabilities`, and the control plane stores it on the agent. When the filter holds only the profile tag, the control plane sends it in `tags`, which every agent reads. A filter with more tags goes to `all_tags` for an agent with the capability only. For any other agent the builder refuses the job with the reason `agent_capability_missing`.
- A `forget` step that selects no snapshot can hide a wrong filter. The step result shows it, and the control plane stores the job as `succeeded_with_warnings` with the message `forget_selected_nothing`.
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
| Filter on the host name of the agent and on the paths | An Admin can rename an agent. The paths of a profile can change. The filter then misses the snapshots that the profile made earlier. |
| Filter on host and paths only | Two profiles on one host with overlapping paths share snapshots, so one retention could remove the snapshots of the other profile. |
| One merged retention for the repository | A union of the `keep` rules keeps more than a profile asked for, and an intersection could delete data that a profile wants to keep. |
| Refuse conflicting retention at save time | Without an exact key the control plane cannot tell whether two profiles conflict. |
| A `prune` job behind the `forget` job | The index of the active job allows one exclusive job for each repository, so the second job could not wait in the queue. |
| A maintenance plan row for each repository | A plan binds a profile and an agent, and a repository task has neither. |
