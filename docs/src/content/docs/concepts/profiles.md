---
title: Profile options
description: The backup options and the limits of a profile.
---

A profile holds the repository, the paths, the excludes, the tags, the backup options and the limits of a backup (FR-PF-01). This page lists the options that change how restic runs.

## Backup options

| Option | Effect |
|---|---|
| Use a file system snapshot (VSS) | Passes `--use-fs-snapshot`. Windows hosts only. |
| Stay on one file system | Passes `--one-file-system`. |
| Exclude cache directories | Passes `--exclude-caches`. |
| Upload limit, download limit | Pass `--limit-upload` and `--limit-download`. The unit is KiB each second. |

## Limits

| Setting | Default | Meaning |
|---|---|---|
| Stop after no progress | 60 minutes | The agent stops a step that makes no progress for this time. From 5 to 10080. |
| Maximum duration | 48 hours | The agent stops the job after this time, whatever its progress. From 0 to 43200. Zero turns the limit off. |

See [Jobs](../jobs/#time-limits) for how the agent measures progress.
