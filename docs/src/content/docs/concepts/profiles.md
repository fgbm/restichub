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
| Exclude cloud-only files | Passes `--exclude-cloud-files`. Windows hosts only. |
| Upload limit, download limit | Pass `--limit-upload` and `--limit-download`. The unit is KiB each second. |

### Cloud-only files

A cloud service such as OneDrive can keep a placeholder on the disk. The file content stays in the cloud. When restic reads such a file through VSS, Windows answers "Access to the cloud file is denied". The backup then ends with warnings.

The option "Exclude cloud-only files" makes restic skip these files. The compiler adds the flag for Windows hosts only, so one profile can serve Windows and Linux hosts. A new profile has the option on. An existing profile keeps its old setting. The option needs restic 0.17 or later.

## Limits

| Setting | Default | Meaning |
|---|---|---|
| Stop after no progress | 60 minutes | The agent stops a step that makes no progress for this time. From 5 to 10080. |
| Maximum duration | none | The agent stops the job after this time, whatever its progress. From 1 to 43200. |

See [Jobs](../jobs/#time-limits) for how the agent measures progress.
