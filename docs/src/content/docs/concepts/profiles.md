---
title: Profiles
description: What a profile holds, the sections of the profile editor, and the retention rules.
---

A profile is the definition of a backup: what to copy, what to skip, how to run restic and which snapshots to keep. A [plan](/restichub/concepts/plans/) binds a profile to a host and a [repository](/restichub/concepts/repositories/) with a schedule. A profile holds no repository, so one profile serves many hosts and many repositories.

## Editor sections

The profile editor is one page. Select "Add profile" on the "Profiles" screen, or select the name of a profile. The menu "…" of a profile row holds only "Make a copy" and "Delete profile". Each section shows its state ("On", "Off" or "Needs attention") and a short summary. You can switch a section on or off, and you can use "Expand all" and "Collapse all". The side panel shows "The result": a live summary of the profile. The "Restic commands" button at the top of the page opens a side panel. It shows the restic commands that the profile produces for the selected host and repository, and it has a "Copy" button. The button is available when the profile has paths. The bar at the bottom of the page shows "No changes" or "The changes are not saved", "Cancel" and "Save". When the profile has plans, the bar tells how many plans use it. The new settings apply to their next run.

| Section | What you set |
|---|---|
| Main | The name of the profile. The name is how you find the profile in the list and in plans. A name is any text of 1 to 100 characters, with letters of any script, spaces and emoji. It is unique in the tenant, and the letter case does not matter. |
| Parameters | The names and the kinds of the values that a host or a plan sets, for example a mount point or a password. See [Profile parameters](/restichub/concepts/profile-parameters/). |
| What to back up | One path in each line. The paths for "All systems" apply on every host. The other paths apply only on their own system (Windows, Linux or macOS). |
| Exclusions | Patterns such as `*.tmp`, or paths. One entry in each line. Under "More" you can give a file on the host with a list of patterns ("Exclude list file on the host"). |
| Tags | The tags of the snapshots of this profile. The retention rules and the searches use them. Type a tag and press Enter, or separate tags with a comma or a space. Each tag becomes a chip. Select the cross on a chip to remove the tag. |
| Backup options | VSS (Windows only), which passes `--use-fs-snapshot`. "Stay on one file system" (`--one-file-system`). "Exclude cache directories" that hold a `CACHEDIR.TAG` file (`--exclude-caches`). "Upload limit (KiB/s)" and "Download limit (KiB/s)" (`--limit-upload`, `--limit-download`). |
| Retention | The rules that decide which snapshots stay. See the section below. |
| Repository check | A check after the backup. Without a "Read-data subset", the check reads the repository structure only. This is fast. A share such as `5%` or `1/10` reads that part of the data. A larger share takes more time and traffic. |
| Scripts on the host | Scripts before and after the backup. See [Server scripts](/restichub/guides/scripts/). |
| Run limits | The stall limit and the maximum duration of a job. See the section below. |

A profile has no hooks. Use [server scripts](/restichub/guides/scripts/) to run a command on the host before or after a backup.

The paths, the exclusions, the exclude files and the tags hold variables such as `{{hostname}}` and `{{param.<name>}}`. The profile editor lists them when you type `{{`. See [Template variables](/restichub/reference/variables/).

Only the [maintenance host](/restichub/concepts/repositories/) of a repository runs the `forget` and `prune` steps of that repository.

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

The editor shows a sample of 60 daily snapshots. The last three days hold two snapshots a day. The sample shows which snapshots the current rules keep. The legend marks a snapshot that stays because of a tag as "stays because of a tag".

### Presets

A new profile starts with the Standard preset. You can pick another preset or change the values.

| Preset | Last | Daily | Weekly | Monthly | Yearly |
|---|---|---|---|---|---|
| Minimum | 3 | 7 | 0 | 0 | 0 |
| Standard | 3 | 7 | 4 | 6 | 0 |
| Long | 3 | 14 | 8 | 12 | 5 |

## Job limits

The section "Run limits" protects against jobs that hang or run too long. The stall limit always applies. An empty field leaves the value to the server: a new profile gets the default, and a profile that you edit keeps its stored value.

| Field | Unit | Default | Allowed values | Variable for the whole installation |
|---|---|---|---|---|
| "Stop after no progress (minutes)" | Minutes | 60 minutes | 5 to 10080 | [`RESTICHUB_JOB_STALL_TIMEOUT_SECS`](/restichub/reference/environment-variables/) |
| "Maximum duration (hours)" | Hours | 48 hours | 0 to 720. 0 means no limit. | [`RESTICHUB_JOB_MAX_DURATION_SECS`](/restichub/reference/environment-variables/) |

If restic makes no progress for longer than the first limit, the agent stops the step and the job fails as stalled. A `check` or `prune` step waits for at least 6 hours, even when the profile sets a shorter limit. A `backup` step with an upload limit waits at least twice the time to upload a pack of 128 MiB at that limit. A narrow link therefore does not stop it while it uploads. A job that runs longer than the second limit stops even when it makes progress. The variable sets the default of a new profile. A value in the profile overrides it. The [Jobs](/restichub/concepts/job-lifecycle/#time-limits) page describes how the agent measures progress. The job detail shows the limits of each job.
