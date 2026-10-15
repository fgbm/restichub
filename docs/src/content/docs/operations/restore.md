---
title: Restore
description: How to list the snapshots of a repository, browse a snapshot and restore files to the same agent or to a different agent.
---

The Restore screen lists the snapshots of a repository, shows the files of a snapshot and restores them. An Operator or an Admin can use it. A Viewer cannot.

The data of a restore goes from the repository to the target agent. It does not go through the control plane (ADR-0003). The target agent gets the secrets of this repository for the restore job only. Each delivery writes the audit record `secret_delivered`. The start of a restore writes the audit record `restore_start`. The record names the repository, the snapshot, the target agent and the target path.

## Requirements

- The repository must be initialized.
- The agent that you select must be online.
- The agent must announce the capability `restore-v1`. This capability is in the agent packages that include the Restore screen. An agent without it can list snapshots, but the control plane refuses the file tree and the restore with the reason `agent_capability_missing`. Update the agent package to fix this.
- A repository with a local path accepts its maintenance agent only. The data stays on that host.

## List the snapshots

1. Open **Restore**.
2. Select a repository. The screen shows the newest stored list and its age.
3. Enter a host, a tag or a path. Enter a date range if you need one.
4. Click **Refresh**. An agent runs `restic snapshots` with the filters, and the list updates.

The control plane applies the date range to the list. restic applies the host, the tag and the path. The list agrees with `restic snapshots --json` for the same filters. When a repository holds more snapshots than a result can carry, the screen shows a warning and keeps the newest snapshots.

## Browse a snapshot

Select a snapshot to open its file tree. The screen shows one directory at a time. The agent reads the directory and returns a page of at most 1000 entries. Select **Load more** to read the next page. A directory with one million files shows its first page in less than 5 seconds.

The control plane keeps a page for 10 minutes. If you open the same directory again in that time, the screen shows the stored page and no job runs. The Jobs list does not show the browse jobs.

## Restore files

1. Select the files and directories that you need in the tree. If you select nothing, the restore uses the whole snapshot.
2. Click **Restore**.
3. Select the target agent. The default is the maintenance agent of the repository.
4. Enter the target path. The path must be absolute, for example `/srv/restore` or `D:\restore`.
5. Select what to do with a file that exists in the target.
6. Click **Restore**. The screen opens the job.

The job detail shows the progress while the restore runs. After it ends, the detail shows how many files and bytes the agent restored and how many it skipped.

| Option | Meaning |
|---|---|
| Overwrite: `always` | Replace each file that exists. |
| Overwrite: `if-changed` | Replace a file whose content differs. |
| Overwrite: `if-newer` | Replace a file that is older than the copy in the snapshot. |
| Overwrite: `never` | Keep each file that exists. |
| Dry run | Check the restore. The agent writes no file and changes no file. |
| Verify | After the restore, the agent reads the restored files again and compares them with the snapshot. |

restic does not accept an include list and an exclude list in one restore. The screen restores the files that you select, and the API refuses a request that has both lists.

A restore to a system path, for example `/etc`, `/usr`, `C:\Windows` or `C:\Program Files`, can break the target host. The screen shows a warning. The control plane does not block the restore.

## API

| Method and path | Role | Use |
|---|---|---|
| `POST /api/v1/repositories/{id}/snapshots` | Operator | Starts a job that lists the snapshots. |
| `GET /api/v1/repositories/{id}/snapshots` | Viewer | Reads the list of a job (`job`), or the newest stored list. The parameters `from` and `to` filter by date. |
| `POST /api/v1/repositories/{id}/ls` | Operator | Starts a job that lists one directory of a snapshot. A stored page answers with HTTP 200. |
| `GET /api/v1/repositories/{id}/ls` | Operator | Reads the page of a job (`job`). |
| `POST /api/v1/repositories/{id}/restore` | Operator | Starts a restore and returns the job. |

An offline agent gives HTTP 409 `agent_offline`. An agent that is too old gives HTTP 409 `agent_capability_missing`. A repository of another organization gives HTTP 404.

## Related

- [Agent jobs](agent-jobs/)
- ADR-0035 in the design folder.
