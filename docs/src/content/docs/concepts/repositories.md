---
title: Repositories
description: Store the backups of the hosts, and keep the repository clean with the maintenance host.
---

A repository is the place where restic stores the backups. The control plane keeps the address of the repository and its secrets. The agent receives the secrets for one job only (ADR-0003).

## Maintenance

Each repository has one maintenance host, the maintenance agent. Only this host runs `forget`, `prune` and `check` (FR-RP-08). A backup host never receives these steps. Use a dedicated host for the maintenance.

A repository without a maintenance host gets no maintenance job. The repository dialog shows a warning, and the control plane creates no `forget`, `prune` or `check` job for the repository.

The control plane creates the maintenance jobs from two schedules of the repository:

| Task | Default | Job |
|---|---|---|
| Retention | Every week, Sunday at 03:00 | One `forget` step for each profile and host, then one `prune` step. |
| Check | Every month, on the first day at 04:00 | One `check` step. |

Each schedule has a cron expression and an enabled flag. Both schedules use one time zone, UTC by default. The repository dialog previews the next fire times. A change of a schedule starts from the next fire time after the change.

The control plane creates a maintenance job only when the repository is initialized and no other exclusive job is waiting or running on it. If another exclusive job holds the repository, the schedule stays due and the control plane creates the job after that job ends. A missed fire time older than 24 hours is skipped.

### Run now

An Operator or an Admin can run the retention or the check at once from the repository row. The job follows the same rules as a scheduled job. The retention needs a profile with a retention that a plan runs.

### Barriers

A maintenance job is exclusive (FR-SC-04). It starts after the running backups of the repository end. The backups that the control plane queues after it wait until it ends (AC-SC-03). The [Jobs](/restichub/concepts/jobs/) page shows the maintenance jobs with the backups.

### Retention of several profiles

Several profiles can use one repository. The retention of a profile removes only the snapshots of that profile (ADR-0034):

- The control plane adds the tag `restichub-profile-<profile id>` to every backup of a profile.
- The `forget` step of a profile filters on this tag, on the host name of the agent and on the backup paths of the agent. It uses the grouping of the profile. A profile without a grouping uses `host,paths`.
- A filter in the retention of the profile does not apply to the maintenance job.
- A profile that runs on two hosts gets one `forget` step for each host.

A snapshot that was made before the tag existed has no tag, and no retention selects it. Remove such snapshots by hand. A profile with a retention and without a plan gives no `forget` step.

### Check

The `check` step reads the data subset of the first profile of the repository that sets one. If no profile sets a subset, the check reads the structure only.

## Roles

| Role | Right |
|---|---|
| Admin | Read and change the maintenance host and the schedules. |
| Operator | Read the maintenance and run a task now. |
| Viewer | Read the maintenance. |
