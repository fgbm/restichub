---
title: Create a profile and a plan
description: Create the backup definition, attach it to a host with a schedule, and run the first job.
---

A profile defines what to back up. A plan runs a profile on a host with a schedule. This page shows how to create one profile and one plan for the first backup.

## Before you start

- The host has the status "Online" on the screen "Hosts". See [Add a host](/restichub/getting-started/add-a-host/).
- The repository has the status "Ready" on the screen "Repositories". See [Add a repository](/restichub/getting-started/add-a-repository/).
- The host can reach the repository. For a local repository, the host is the repository host. See [Repositories](/restichub/concepts/repositories/).

## Create a profile

1. Open the screen "Profiles" and select "Add profile".
2. In the section "Main", enter a name in "Profile" and select the repository in "Repository".
3. In the section "What to back up", enter one path in each line. A path in "All systems" applies to every host. A path in "Windows", "Linux" or "macOS" applies only to that system.
4. In the section "Exclusions", enter the patterns and the paths to skip. This step is optional.
5. In the section "Retention", select a preset: "Minimum", "Standard" or "Long". The preset sets the rules that decide which snapshots stay. See [Retention rules](/restichub/concepts/profiles/#retention-rules).
6. Select "Save".

Result: the list shows the profile, and the side panel "The result" shows a summary of the profile.

## Create a plan

1. Open the screen "Plans" and select "Add plan".
2. In the section "Basics", enter a name in "Plan".
3. In the section "What to back up", select the profile in "Profile" and the host in "Host".
4. In the section "Schedule", set the "Cron expression" and the "Time zone". You can also select a preset: "Hourly", "Daily at 02:00" or "Sundays at 03:00". The switch "Run on schedule" stays on.
5. In the section "Recovery point objective", enter the hours after which the backup is overdue. This step is optional. The default is the schedule interval plus a margin. See [Recovery point objective](/restichub/guides/alerts/#recovery-point-objective).
6. Select "Save".

Result: the list shows the plan and its next run.

The control plane compiles the profile for the operating system of the host. The control plane refuses a profile without a path for that operating system.

## Run the plan now

1. Open the menu "…" of the plan and select "Run now".
2. Open the screen "Jobs". The job has the status "Succeeded" when the agent finishes.

See [Watch and cancel jobs](/restichub/guides/jobs/).

## Related pages

- [Profiles](/restichub/concepts/profiles/)
- [Plans](/restichub/concepts/plans/)
- [Repositories](/restichub/concepts/repositories/)
- [Alerts and notification channels](/restichub/guides/alerts/)
