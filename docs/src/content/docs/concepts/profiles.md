---
title: Profiles
description: What a profile holds, the sections of the profile editor, and the retention rules.
---

A profile is the definition of a backup: what to copy, what to skip, how to run restic and which snapshots to keep. A [plan](/restichub/concepts/plans/) assigns a profile to a host with a schedule. The profile belongs to one [repository](/restichub/concepts/repositories/).

## Editor sections

The profile editor is one page. Each section shows its state (on, off or needs attention) and a short summary. The side panel shows a live summary and the restic commands that the profile produces.

| Section | What you set |
|---|---|
| Basics | The name of the profile and its repository. The name is how you find the profile in the list and in plans. |
| What to copy | One path in each line. The paths for all systems apply on every host. The other paths apply only on their own system (Windows, Linux or macOS). |
| Exclusions | Patterns such as `*.tmp`, or paths. One entry in each line. You can also give the path of a file on the host that holds a list of patterns. |
| Backup options | Snapshot tags and VSS (Windows only). Stay on one file system. Skip the cache directories that hold a `CACHEDIR.TAG` file. Limit the upload and download speed in KiB/s. |
| Repository check | A check after the backup. Without a read share, the check reads the repository structure only. This is fast. A share such as `5%` or `1/10` reads that part of the data. A larger share takes more time and traffic. |
| Scripts on the host | Scripts before and after the backup. See [Server scripts](/restichub/operations/scripts/). |
| Execution limits | The stall limit and the maximum duration of a job. See the section below. |

The retention rules are part of the profile. Only the [maintenance host](/restichub/concepts/repositories/) of the repository runs the `forget` and `prune` steps.

## Retention rules

A snapshot stays if at least one rule keeps it. The rules do not subtract. They add up.

A value of 0, or an empty field, switches a rule off. If no rule is on, the control plane deletes no snapshot and does not start `prune`.

| Rule | What it keeps |
|---|---|
| Last | The N newest snapshots. |
| Hourly | The newest snapshot of each of the last N hours that have snapshots. |
| Daily | The newest snapshot of each of the last N days that have snapshots. |
| Weekly | The newest snapshot of each of the last N weeks (Monday to Sunday) that have snapshots. |
| Monthly | The newest snapshot of each of the last N calendar months that have snapshots. |
| Yearly | The newest snapshot of each of the last N calendar years that have snapshots. |
| Within | All snapshots that are not older than the given time span. The span starts at the newest snapshot, not at today. Examples: `30d`, `6m`, `1y2m7d3h`. |
| Keep tags | A snapshot with one of these tags always stays. |

The hourly, daily, weekly, monthly and yearly rules count only the intervals that have snapshots. The rule "daily 5" keeps five days that have snapshots. It does not mean the last five calendar days.

The rules apply to each group of snapshots separately. Then the copies of different hosts or paths do not push each other out. Without a choice, the group is the host and the paths. A filter can limit the rules to some hosts, tags or paths. Snapshots outside the filter stay.

The editor shows a sample of 60 daily snapshots and which of them the current rules keep.

### Presets

A new profile starts with the Standard preset. You can pick another preset or change the values.

| Preset | Last | Daily | Weekly | Monthly | Yearly |
|---|---|---|---|---|---|
| Minimum | 3 | 7 | 0 | 0 | 0 |
| Standard | 3 | 7 | 4 | 6 | 0 |
| Long | 3 | 14 | 8 | 12 | 5 |

## Job limits

The limits protect against jobs that hang or run too long. They always apply. An empty field uses the default.

| Limit | Default | Environment variable for the whole stack |
|---|---|---|
| No progress | 60 minutes | `RESTICHUB_JOB_STALL_TIMEOUT_SECS` |
| Maximum duration | 48 hours | `RESTICHUB_JOB_MAX_DURATION_SECS` |

If the agent reports no progress for longer than the first limit, the control plane stops the job as stalled. A job that runs longer than the second limit stops even when it makes progress. The environment variable sets the default. A value in the profile overrides it.
