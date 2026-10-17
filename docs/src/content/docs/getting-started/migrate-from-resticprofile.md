---
title: Migrate from resticprofile
description: How to move the backups of a resticprofile configuration to ResticHub profiles, plans and repositories.
---

resticprofile keeps the settings of restic in a configuration file on each host. ResticHub keeps them on the control plane. This page maps the settings of resticprofile to ResticHub. ResticHub has no import function. You create the objects in the web UI.

## What stays the same

- The restic repository stays as it is. ResticHub runs restic on the host, so the snapshots stay readable.
- The repository password stays the same. Enter it in ResticHub when you add the repository.
- Do not run `restic init` again.

## Map the settings

| resticprofile | ResticHub |
|---|---|
| `repository`, `password-file`, backend variables | A [repository](/restichub/concepts/repositories/). The control plane encrypts the password and the credentials. |
| A profile section | A [profile](/restichub/concepts/profiles/). |
| `backup.source` | "What to back up" |
| `backup.exclude`, `exclude-file` | "Exclusions" |
| `backup.tag` | "Tags" |
| `backup.one-file-system`, `exclude-caches`, `limit-upload`, `limit-download`, `use-fs-snapshot` | "Backup options" |
| `retention` | "Retention". The presets and the rules replace the keys `keep-*`. |
| `check` | "Repository check" |
| `run-before`, `run-after` | "Scripts on the host". See [Server scripts](/restichub/guides/scripts/). |
| `schedule`, a systemd timer or a cron entry | A [plan](/restichub/concepts/plans/) with a cron expression and a time zone. |
| `groups` | One plan for each profile. |
| `inherit` | Create the second profile by hand. ResticHub has no profile inheritance. |

## Procedure

1. Install the control plane. See [Install the control plane](/restichub/getting-started/install-control-plane/).
2. Register each host. See [Register an agent](/restichub/getting-started/add-a-host/).
3. Add the repository. Enter the same backend address and the same password as in resticprofile.
4. Add a profile for each resticprofile profile. Copy the paths, the exclusions and the tags.
5. Set the retention. Choose the preset that is closest to your `keep-*` values, then change the numbers.
6. Add a plan for each profile. Convert the schedule to a cron expression.
7. Select "Run now" on the plan. Read the job log.
8. Check that the new snapshot appears in the repository.
9. Disable the schedule of resticprofile on the host. Remove the systemd timer or the cron entry.

Do not run the old schedule and the new plan together. Two schedules make two backups of the same data.

## Differences to know

- The control plane holds the schedule. A host that is offline misses its runs. The plan setting "Missed runs" decides what happens at the next contact.
- The control plane runs `forget` and `prune` only on the maintenance host of the repository. Choose this host in the repository settings.
- A profile does not hold free restic flags. The profile editor offers only typed options. If you need a flag that the editor lacks, ask in the project issues.
- ResticHub has no hooks. Use a server script. A script runs only on a host where an administrator turned on the trust mode.
- The agent downloads restic by itself. You do not manage the restic binary.

## Related pages

- [Profiles](/restichub/concepts/profiles/)
- [Plans](/restichub/concepts/plans/)
- [Restore](/restichub/guides/restore/)
