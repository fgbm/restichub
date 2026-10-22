# ADR-0051: A retry of restic is not progress

- Status: accepted
- Date: 2026-10-22
- Requirements: FR-AG-05, FR-AG-10
- Refines: ADR-0039

## Context

ADR-0039 gives each running step a stall watchdog and counts every line that restic prints as progress.

A repository that refuses a connection is the largest case of a step without progress. restic retries the request and prints one line for each attempt on the standard error: `Stat(<config/>) returned error, retrying after 1.31s: …`, and then at 2.38, 4.17, 7.42, 13.56 and 37.94 seconds. Each line restarted the stall timeout, so a window of 5 minutes never closed. A backup against a repository that is down ran on until restic ended its retries. The profile setting "Stop after no progress" had no effect.

A `backup` and a `restore` report their progress as status messages on the standard output. On the standard error they report a warning about one file or a retry of the backend.

A `check` and a `prune` print their only output on the standard error, and they work for hours without a status message.

## Decision

- A `backup` and a `restore` report their progress as status messages on the standard output. For these two steps, a line on the standard error is not progress. A plain line on the standard output is progress, and so is a status message whose counters or current files changed.
- For every other step, a line on either stream is progress, as ADR-0039 says.
- The stall timeout of the job still sets the window (ADR-0039).

## Consequences

- A backup against a repository that refuses every request stops with the code `stalled` after the stall timeout of its profile.
- An error about one file no longer holds the watchdog open. A `backup` prints a status message while it works, so a backup that reads files and reports errors keeps the watchdog open.
- The agent keeps the rule of ADR-0039 for `check` and `prune`, because they print no status message.

## Note of 2026-10-22: the counters of a restore, its verify phase, a narrow upload and the lock wait

A `restore` over a target that already holds the snapshot reads and hashes the files that did not change. restic counts those files as skipped. It then grows `files_skipped` and `bytes_skipped` only, and `files_restored` and `bytes_restored` stay at 0 for the whole run. The watchdog read the two restored counters only, so a healthy restore stopped as `stalled` after the stall timeout. The scan of the tree also grows `total_files` and `total_bytes` before restic restores the first file.

- A status message of a step is progress when one of these values changes: `bytes_done`, `files_done`, `bytes_skipped`, `files_skipped`, `total_bytes`, `total_files`, or the current files. The skipped counters restart the watchdog only; the protocol message keeps the restored counters.
- A `restore` with `--verify` prints its last status message and its summary before it verifies the files. The verification prints nothing, on either stream (restic `cmd/restore.go`, `internal/ui/restore/json.go`). Its progress bar is for a terminal only. The agent does not see the files that restic reads there, so it cannot tell a slow disk from a hung one. A wrong guess would stop a restore that wrote every file. The agent therefore gives the verify phase 24 hours, counted from the summary message, or the longer stall timeout of the step. A verification that runs longer stops the step as `stalled`, also when it is healthy. A cancel also stops it. A `restore` job carries no maximum duration, so nothing else does. A verification that needs more than 24 hours needs a larger `RESTICHUB_JOB_STALL_TIMEOUT_SECS` on the agent, up to 7 days. A verification that runs for a week or longer is a planning problem of the backup, not a case that the default supports.
- A `backup` step with an upload limit reports no new `bytes_done` while a pack uploads. The agent raises the window of such a step. It must hold at least twice the time of one pack of 128 MiB at the limit (ADR-0039). A narrow link then does not stop a healthy backup.
- `--retry-lock` waits 4 minutes. The shortest stall timeout of a profile is 5 minutes. restic therefore ends a held lock with exit code 11 before the watchdog stops the step. The step then ends with the code `repository_locked`, and the control plane retries the job. Before this change the two values were equal, and the watchdog fired about one second before restic gave up. The agent accepts `RESTICHUB_JOB_STALL_TIMEOUT_SECS` from 300 seconds for the same reason: a shorter window would stop a step that waits for a lock.
- A `restore` job carries no stall timeout from a profile. The control plane cannot see the value that the agent uses. The card of the job names the default of the agent without a number.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Count a retry line as progress by its text (`returned error, retrying after`) | The text belongs to restic, not to the product. A newer restic can change it, and the watchdog counts the retry as progress again. |
| Ignore the standard error for every step | `check` and `prune` print their only output there. A long `prune` would stop after 6 hours although it advances. |
| Judge the verify phase of a restore by its restored counters | restic prints no counter while it verifies, so the step stops although the disk is only slow. |
| Give the lock wait a grace period before the first status line | The grace is a second time value next to the stall timeout, and the operator cannot see it. A shorter `--retry-lock` keeps one value. |
