---
title: Repositories
description: Store the backups of the hosts, and keep the repository clean with the maintenance host.
---

A repository is the place where restic stores the backups. The control plane keeps the address of the repository and its secrets. The agent receives the secrets for one job only.

## Maintenance

Each repository has one maintenance host, the maintenance agent. Only this host runs `forget`, `prune` and `check`. A backup host never receives these steps. Use a dedicated host for the maintenance.

A repository without a maintenance host gets no maintenance job. The repository dialog shows a warning, and the control plane creates no `forget`, `prune` or `check` job for the repository.

The control plane creates the maintenance jobs from two schedules of the repository:

| Task | Default | Job |
|---|---|---|
| Retention | Every week, Sunday at 03:00 | One `forget` step for each profile and host, then one `prune` step. |
| Check | Every month, on the first day at 04:00 | One `check` step. |

Each schedule has a cron expression and an enabled flag. Both schedules use one time zone, UTC by default. The repository dialog previews the next fire times. A change of a schedule starts from the next fire time after the change.

The control plane creates a maintenance job only when the repository is initialized and no other exclusive job is waiting or running on it. If another exclusive job holds the repository, the schedule stays due and the control plane creates the job after that job ends. A missed fire time older than 24 hours is skipped.

### Run now

An Operator or an Admin can run the retention or the check at once from the repository row. The job follows the same rules as a scheduled job. The retention needs a profile with a retention that a plan of an active agent runs.

### Barriers

A maintenance job is exclusive. It starts after the running backups of the repository end. The backups that the control plane queues after it wait until it ends. The [Jobs](/restichub/concepts/jobs/) page shows the maintenance jobs with the backups.

### Retention of several profiles

Several profiles can use one repository. The retention of a profile removes only the snapshots of that profile:

- The control plane adds the tag `restichub-profile-<profile id>` to every backup of a profile.
- The tag prefix `restichub-` is reserved. A profile tag, a keep tag and a retention filter tag cannot start with it.
- Each profile with a retention gets one `forget` step. The step filters on the profile tag. It uses the grouping of the profile. A profile without a grouping uses `host,paths`, so each host and each set of paths keeps its own policy.
- The filter in the retention of the profile narrows the step. A snapshot must have the profile tag and all the filter tags. The filter paths and the filter hosts narrow the step more. A plan agent whose host name is not in the filter hosts gives no step. The host name compares with the host name that restic records in the snapshot. A template is not allowed in the filter hosts.
- The filter tags always mean AND with the profile tag. The `forget` step that follows a backup uses the same rule.
- The step always groups by host, because a profile can run on several agents. If the grouping of the profile has no host, the control plane adds it, and each host keeps its own snapshots.
- An agent from an older release does not know the filter with more than one tag. A retention with filter tags fails with the reason `agent_capability_missing` until you update that agent. The control plane uses the capabilities that the agent announced on its current connection. If you install an older package of the agent, the control plane notices at the next connection and does not send the filter. This is safe: you can downgrade an agent at any time.
- A `forget` step that selects no snapshot gives the job status `succeeded_with_warnings`. The reason is `forget_selected_nothing`.

A snapshot that was made before this release has no profile tag, and no retention selects it. To put such snapshots under the retention, add the tag once. Run this command on the host of the agent for each profile, with the host name and the path of the backup:

```sh
restic tag --add restichub-profile-<id> --host <host> --path <path>
```

Otherwise, remove such snapshots by hand. A profile with a retention and without a plan gives no `forget` step.

### Check

The `check` step reads the largest data subset that a profile of the repository sets. If two subsets are equal, the oldest profile wins. If no profile sets a subset, the check reads the structure only.

## Roles

| Role | Right |
|---|---|
| Admin | Read and change the maintenance host and the schedules. |
| Operator | Read the maintenance and run a task now. |
| Viewer | Read the maintenance. |
