# ADR-0035: Restore and snapshot browsing

- Status: accepted
- Date: 2026-10-15
- Requirements: FR-RS-01, FR-RS-02, FR-RS-03, FR-RS-04, FR-PR-02, FR-PR-09, FR-AU-01, AC-RS-01, AC-RS-02, AC-RS-03, AC-RS-04, AC-PR-03

## Context

[ADR-0003](0003-direct-repository-access.md) says that backup data never goes through the control plane. A restore must follow the same rule: the target agent reads the repository directly. The steps `ls` and `restore` exist in the protocol, but the agent rejected them. The Restore screen must show the snapshots and the tree of a snapshot, and a tree can hold one million files.

## Decision

- The agent announces the capability `restore-v1` in `AgentHello.capabilities`. The control plane refuses an `ls` or `restore` job for an agent without it, with the reason `agent_capability_missing`. The API checks the live connection when it creates the job, and the dispatcher checks it again at the send (FR-PR-09). An old agent also rejects the step kind, but the control plane never relies on that.
- Three job kinds carry the Restore screen: `snapshots`, `ls` and `restore`. The row of the job holds no command (ADR-0010). The table `job_requests` holds the typed request of the job. The queue builder turns it into one typed step. The step carries the repository of the job only, so the agent gets the secrets of that repository and no other (AC-PR-03). Each delivery writes the audit record `secret_delivered` (ADR-0017).
- A restore starts only from the Operator role or higher (requirements, section 2). The API writes the audit record `restore_start` with the repository, the target agent, the snapshot and the target path. A restore to a system path is a warning in the user interface. It is not a block.
- A restore job runs on the agent that the user selects. A repository with a local path accepts its maintenance agent only, because the data lives on that host. The control plane never relays data.
- `include` and `exclude` patterns cannot be used together, because restic refuses both. The API and the shared validator refuse a restore that has both. The overwrite mode is a closed set: `always`, `if-changed`, `if-newer` and `never`. The target path must be absolute. A snapshot identifier is 8 to 64 lower-case hexadecimal characters. `latest` is not allowed (see Restore roots and limits).
- The agent parses the restic `--json` output of a restore. The `status` messages become progress, and the `summary` message becomes a typed `RestoreSummary`. The job detail shows the summary. A dry run changes no file.

### Browsing

- A `snapshots` job lists the snapshots with the filters host, tag and path. The date filter is applied by the control plane when it serves the list. The control plane stores the list in `job_listings` after it bounded and cleaned each value that the agent sent. `GET /api/v1/repositories/{id}/snapshots` returns the list of a job, or the newest stored list with its age.
- An `ls` job reads one directory level. The agent runs `restic ls` for the directory, skips the first `offset` entries and keeps at most `limit` entries (the default is 1000, the maximum is 5000). The listed directory is not an entry of itself. When the page is full and more entries follow, the agent stops restic. A result therefore holds one page only. The first page of a directory with one million files comes in less than 5 s (AC-RS-02). The root is the path `/`, because `restic ls` without a directory lists the whole tree.
- The control plane answers a repeated page request from the stored copy for 10 minutes and starts no job. The sweep deletes the page listings after one hour. It deletes the snapshot listings after one hour and the finished browse jobs after seven days. The job list hides the browse jobs.
- A browse job holds no slot of its agent and no limit of its repository. The agent counts a browse job in its own small room (see Restore roots and limits). A person waits for it, and a long backup must not hold the screen. It still follows the barrier of an exclusive job (FR-SC-04). The index of the active jobs of a repository excludes the kinds `snapshots`, `ls` and `restore`, like `backup`.
- A browse job expires after two minutes in the queue. An offline agent gives HTTP 409 `agent_offline` at once.

## Restore roots and limits (2026-10-15)

A restore runs as root on Linux and as LocalSystem on Windows. A target path from the control plane could therefore replace the identity of the agent. It could also replace the CA chain, the journal, the binary or a system file. A compromised control plane or a stolen Operator token must not have this power. This decision follows [ADR-0004](0004-hook-allow-list.md): the host administrator decides, and the control plane and the jobs cannot change it.

### Decision

- The agent restores only below a restore root. Only root or Administrators configure the roots. The default root is `/var/lib/restichub-restore` on Linux and `C:\ProgramData\ResticHub\Restore` on Windows. `install-service` creates it with access for the service account only (SYSTEM and Administrators on Windows).
- An administrator adds a root in the file `restichub-agent-restore-roots.txt`. The file lies next to the configuration directory, so a swap of the identity never removes it. It holds one absolute path on each line. A line that starts with `#` is a comment. The agent reads the file only when it passes the checks of the install location (T20). It must be a regular file that root or Administrators own. No other account can change it. The agent ignores any other file and logs a warning. The roots are the default root and the roots of the file. A job and the control plane cannot change them. A root of `/` allows a restore to any directory that is not equal to, an ancestor of or a descendant of a protected path. The target `/` is an ancestor of every protected path, so an in-place restore through `/` is not possible. A root must belong to root or Administrators. The agent ignores a root when the root or an existing ancestor is owned by another account or is writable by a group or by other accounts, because that account could replace a directory with a link. `install-service` refuses such a root. The documentation says so.
- Independent of the roots, the agent always refuses a target that overlaps its own files. The agent resolves symbolic links first: it takes the real path of the deepest existing ancestor and adds the missing names. It then refuses a target that equals, lies inside, or is an ancestor of a protected path. The protected paths are these. The configuration directory (identity, journal and roots file). The cache directory. The directory of the agent and restic binaries. The hook allow-list. A link that points nowhere is refused. A path with a `.` or `..` component is refused. On Windows the check ignores case and reads the verbatim prefix. It refuses a UNC path, an alternate data stream, and a name that ends in a dot or a space.
- The agent checks the target when it accepts the job. It checks again before the restore step starts, because a link can change in between. It rejects the job with the reason `RESTORE_TARGET_DENIED`. The step then does not run.
- The agent reports its roots in `AgentHello.restore_roots`. The control plane keeps them with the agent. The user interface suggests the first root as the target. The control plane reads the roots of the live connection and refuses a target outside them with HTTP 409 `restore_target_denied` before a job exists. It also refuses a path that does not belong to the operating system of the agent with HTTP 400 on the field `target`. The check of the agent is the authority. The check of the control plane is only the early answer.
- The systemd unit lists every root in `ReadWritePaths`, because `ProtectSystem=strict` makes the file system read-only. `install-service` writes the list, so the administrator runs it again after a change of the roots file. The path of a root must exist at that time. A restore into the default root works with the other settings of the unit.
- The restore step never uses the form `snapshot:subfolder`, because the snapshot identifier is hexadecimal only.

### Limits

- A restore and an `ls` need a concrete snapshot identifier. `latest` moves between two reads, so a screen that shows one snapshot could restore another one.
- The overwrite mode of a restore must be set. The shared validator and the agent refuse `OVERWRITE_MODE_UNSPECIFIED`, so the default of restic never decides.
- `dry_run` together with `verify` is refused in the API, in the shared validator and in the user interface, because restic stops with an error.
- The user interface escapes the characters `*`, `?` and `[` of a path that the user selects in the tree. It also escapes a leading `!`. The path then matches itself only. Each character stands in a class (`[*]`), because a backslash is a path separator for restic on Windows. A `]` is already literal. A table test with real restic checks the files `a.txt`, `b.txt` and `[ab].txt`.
- An identical request that still waits or runs (same agent, repository and request) gets the existing job. The control plane refuses a snapshot list or a page when one actor sends more than 60 of them in one minute (HTTP 429). One agent holds at most 8 browse jobs that wait or run (HTTP 429). The rate limit of an actor lives in the memory of one control plane instance, so each instance counts it on its own. The cap of an agent is a database rule: the request locks the agent row before it counts, so instances share it.
- The agent runs at most two browse jobs at the same time, in a room of their own. They do not count in the limit of four jobs, so a scheduled backup never gets `Busy` because someone browses.
- A snapshot list lives one hour in the control plane, because it holds host names and paths.
- A `snapshots` or `ls` job with an explicit `agent_id` sends the repository keys to that agent. The audit record `browse_start` names the repository and the agent. The threat model describes the risk.

## Consequences

- The data of a restore goes from the repository to the target agent. The control plane sees the progress, the summary and the log only.
- The size of a step result stays below the limit of the stream for any directory (AC-RS-02).
- A new agent package is needed for the restore. An older agent keeps working for backups and for the snapshot list.
- The table `job_listings` can grow. The sweep bounds it by age.
- A restore does not work on a host until the agent package with restore roots is installed and the service is installed again. The default root works without a change of the configuration.
- An administrator who wants to restore to a system directory adds the root `/` on purpose. The refusal of the agent files stays, and the target `/` itself stays refused.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| One `ls` step that sends the whole tree in many messages | The control plane would store or relay one million entries, and the first page would wait for the full tree. |
| A cursor token for the pages | The agent stays without state between pages, so an offset is enough. A page of a directory is cheap, because `restic ls` of one directory reads one tree. |
| The control plane sets the allowed paths | A compromised control plane would widen them. The rule must come from the host, like the hook allow-list. |
| Block system paths in a list on the agent | A list of bad paths is never complete. A list of good paths (roots) fails closed. |
| Relay the restored data through the control plane | It breaks ADR-0003 and AC-RS-03. |
| Rely on the old agent to reject the new step kinds | A half-understood job must never run. The capability makes the refusal explicit and gives the user a reason. |
