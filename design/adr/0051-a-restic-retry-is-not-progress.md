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

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Count a retry line as progress by its text (`returned error, retrying after`) | The text belongs to restic, not to the product. A newer restic can change it, and the watchdog counts the retry as progress again. |
| Ignore the standard error for every step | `check` and `prune` print their only output there. A long `prune` would stop after 6 hours although it advances. |
