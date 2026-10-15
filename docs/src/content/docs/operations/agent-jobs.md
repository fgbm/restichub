---
title: Agent jobs
description: What the agent runs for a job, how it reports the result, and how bandwidth limits and priority work.
---

The agent runs the steps of a signed job with the restic binary of its package. The agent starts no job by itself. The control plane sends each job (ADR-0002).

## Steps that the agent runs

| Step | restic command | Result that the agent reports |
|---|---|---|
| `backup` | `restic backup` | Files new, changed and unmodified, bytes added, snapshot ID, duration |
| `check` | `restic check` | Number of errors, repair suggestion |
| `snapshots` | `restic snapshots` | List of snapshots |
| `ls` | `restic ls` | One page of the entries of one directory of a snapshot |
| `restore` | `restic restore` | Files and bytes restored and skipped, duration |
| `forget` | `restic forget` | Snapshots that the rule kept and removed |
| `prune` | `restic prune` | Exit code and log |
| `unlock` | `restic unlock` | Exit code and log |
| `unlock_all` | `restic unlock --remove-all` | Exit code and log |
| `stats` | `restic stats` | Sizes and number of snapshots |
| `test_access` and `init` | `restic cat config` and `restic init` | Exit code |

The `unlock` step removes stale locks only. Only the `unlock_all` step removes all locks, including live locks. The control plane creates an `unlock_all` step only from an Admin action with a confirmation.

The agent announces the capability `restore-v1`, which shows that it runs the `ls` and `restore` steps. See [Restore](restore/). The capability `hooks-v1` shows that it runs the `hook` step from its allow-list. See [Allow-list hooks](hooks/). The capability `scripts-v1` shows that it runs the `script` step on a host with the trust mode. See [Server scripts](scripts/).

The agent builds the restic arguments from the typed fields of the step. It puts each positional value after `--` and writes each option as `--flag=value`. The agent adds `--json` to the commands that print JSON. A value from a job never becomes an option (ADR-0021).

## Step status

The agent maps the restic exit code to the status of the step.

| Exit code | Status |
|---|---|
| 0 | `success` |
| 3 | `warning` |
| 11 | `retry` |
| 10 and 12 | `config_error` |
| 130 | `cancelled` |
| Any other code | `failed` |

A `warning` step does not stop the job. Any other status except `success` stops the job. The job result holds the status of the step that stopped the job. A job that you cancel or that exceeds its timeout stops the restic process in two stages. The agent sends a graceful stop first. After a grace period it kills the process tree and reports `possible_stale_lock`.

## Progress and logs

While a backup runs, the agent sends a progress message at most once in 2 seconds. A progress message holds the files and bytes done, the percentage and the files that restic processes at that moment.

The agent sends the log lines of restic in chunks. The log of a job has a limit of 1 MiB. After the limit the agent adds one line that says that the log is truncated, and drops the rest. The agent limits each line to 64 KiB.

The agent removes secrets from every log line before it sends the line. It replaces the repository password, the backend credentials and the user information of the repository address with `***`.

## When the control plane is not available

The agent finishes a job when the connection to the control plane is down. It keeps the result of every step, the job result and the last 64 KiB of the log in its journal. The journal holds no secrets. When the connection comes back, the agent sends the stored result at once. The journal copy of a step keeps the summary of a backup, a check and the stats. It does not keep the list of snapshots, the page of an `ls` step or the forget result.

## Bandwidth and priority

A `backup` step can set an upload limit and a download limit in kibibytes each second. The agent passes them to restic as `--limit-upload` and `--limit-download`. The value 0 means no limit.

A `backup`, `check`, `forget` or `prune` step can set the low priority.

- On Linux, the agent starts restic with nice value 10 and the best-effort I/O class at level 7.
- On Windows, the agent starts restic in the `BELOW_NORMAL_PRIORITY_CLASS` priority class.

## Related

- [Register an agent](register-an-agent.md)
- [ADR-0011](https://github.com/fgbm/restichub/blob/main/design/adr/0011-restic-cli-integration.md)
- [ADR-0021](https://github.com/fgbm/restichub/blob/main/design/adr/0021-restic-argument-construction.md)
