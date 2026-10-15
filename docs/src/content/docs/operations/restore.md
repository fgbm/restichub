---
title: Restore
description: How to list the snapshots of a repository, browse a snapshot and restore files to the same agent or to a different agent.
---

The Restore screen lists the snapshots of a repository, shows the files of a snapshot and restores them. An Operator or an Admin can use it. A Viewer cannot.

The data of a restore goes from the repository to the target agent. It does not go through the control plane (ADR-0003). The target agent gets the secrets of this repository for the restore job only. Each delivery writes the audit record `secret_delivered`. The start of a restore writes the audit record `restore_start`. The record names the repository, the snapshot, the target agent and the target path. A snapshot list or a tree page that starts a job writes the audit record `browse_start`. It names the repository and the agent that reads it, because that agent gets the repository secrets.

## Requirements

- The repository must be initialized.
- The agent that you select must be online.
- The agent must announce the capability `restore-v1`. This capability is in the agent packages that include the Restore screen. An agent without it can list snapshots, but the control plane refuses the file tree and the restore with the reason `agent_capability_missing`. Update the agent package to fix this.
- A repository with a local path accepts its maintenance agent only. The data stays on that host.
- The target path must lie inside a restore root of the agent (see [Restore roots](#restore-roots)).

## List the snapshots

1. Open **Restore**.
2. Select a repository. The screen shows the newest stored list and its age.
3. Enter a host, a tag or a path. Enter a date range if you need one.
4. Click **Refresh**. An agent runs `restic snapshots` with the filters, and the list updates.

The control plane applies the date range to the list. restic applies the host, the tag and the path. The list agrees with `restic snapshots --json` for the same filters. When a repository holds more snapshots than a result can carry, the screen shows a warning and keeps the newest snapshots.

## Browse a snapshot

Select a snapshot to open its file tree. The screen shows one directory at a time. The agent reads the directory and returns a page of at most 1000 entries. Select **Load more** to read the next page. A directory with one million files shows its first page in less than 5 seconds.

The control plane keeps a page for 10 minutes. If you open the same directory again in that time, the screen shows the stored page and no job runs. A request that is the same as a job that still waits or runs gets that job. The control plane keeps a snapshot list for one hour. The Jobs list does not show the browse jobs.

One person can send 60 list and page requests in one minute. Each control plane instance counts this limit on its own. One agent holds at most 8 waiting browse jobs. A further request gives HTTP 429 `rate_limited`. The agent runs at most two browse jobs at the same time, and they never make a backup wait.

## Restore files

1. Select the files and directories that you need in the tree. If you select nothing, the restore uses the whole snapshot.
2. Click **Restore**.
3. Select the target agent. The default is the maintenance agent of the repository.
4. Check the target path. The screen suggests a new folder in the first restore root of the agent, for example `/var/lib/restichub-restore/a1b2c3d4`. The path must be absolute and must lie inside a restore root.
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
| Verify | After the restore, the agent reads the restored files again and compares them with the snapshot. You cannot use it with a dry run, because a dry run writes no file. |

restic does not accept an include list and an exclude list in one restore. The screen restores the files that you select, and the API refuses a request that has both lists.

The screen escapes the characters `*`, `?` and `[` in a path that you select, so the path matches itself only. A snapshot is selected by its identifier. The screen and the API do not accept `latest`, because it can change between two reads.

## Restore roots

The agent runs as root on Linux and as LocalSystem on Windows. It restores only below a *restore root*. Only root or Administrators set the roots on the host. The control plane and the jobs cannot change them.

| System | Default root |
|---|---|
| Linux | `/var/lib/restichub-restore` |
| Windows | `C:\ProgramData\ResticHub\Restore` |

`install-service` creates the default root with access for the service account only. The agent reports its roots to the control plane. The screen suggests the first root, and the control plane refuses a target outside the roots with HTTP 409 `restore_target_denied`. The agent checks the target again and has the last word. It rejects the job with the reason `restore_target_denied`.

To add a root, write one absolute path on each line of the file `/etc/restichub-agent-restore-roots.txt` (Windows: `C:\ProgramData\ResticHub\Agent-restore-roots.txt`). A line that starts with `#` is a comment. Only root or Administrators can own and write the file. The agent ignores a file that another account can change. Create each root directory, then run `restichub-agent install-service` again. The command adds the roots to `ReadWritePaths` of the systemd unit. Without this step, systemd keeps the root read-only and the restore fails.

A root must be writable only by root or Administrators. The root and each existing parent directory must belong to root or Administrators. No group or other account may write there. The agent ignores a root that does not follow this rule and logs a warning, and `install-service` refuses it. Such an account could replace a directory with a link and lead a restore out of the root.

A root of `/` allows a restore to any other directory, for example `/srv/data`. A file of the agent, a parent of one and a path inside one stay refused. The target `/` is not possible, because it holds the files of the agent. Set the root `/` only when you want this. The screen shows a warning for a system path such as `/etc`.

The agent always refuses a target that is, holds or lies inside its own files. These are the configuration directory, the cache directory, and the directory of the agent and restic binaries. It follows symbolic links first. A root of `/` does not change this rule.

## API

| Method and path | Role | Use |
|---|---|---|
| `POST /api/v1/repositories/{id}/snapshots` | Operator | Starts a job that lists the snapshots. |
| `GET /api/v1/repositories/{id}/snapshots` | Viewer | Reads the list of a job (`job`), or the newest stored list. The parameters `from` and `to` filter by date. |
| `POST /api/v1/repositories/{id}/ls` | Operator | Starts a job that lists one directory of a snapshot. A stored page answers with HTTP 200. |
| `GET /api/v1/repositories/{id}/ls` | Operator | Reads the page of a job (`job`). |
| `POST /api/v1/repositories/{id}/restore` | Operator | Starts a restore and returns the job. |

A restore and a tree page need a snapshot identifier of 8 to 64 lower-case hexadecimal characters. A restore with `dry_run` and `verify` together gives HTTP 400. An offline agent gives HTTP 409 `agent_offline`. An agent that is too old gives HTTP 409 `agent_capability_missing`. A target outside the roots of the agent gives HTTP 409 `restore_target_denied`. A repository of another organization gives HTTP 404.

`GET /api/v1/agents` lists the `restore_roots` of each agent.

## Related

- [Agent jobs](agent-jobs/)
- ADR-0035 in the design folder.
