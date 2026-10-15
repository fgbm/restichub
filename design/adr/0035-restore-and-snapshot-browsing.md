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
- `include` and `exclude` patterns cannot be used together, because restic refuses both. The API and the shared validator refuse a restore that has both. The overwrite mode is a closed set: `always`, `if-changed`, `if-newer` and `never`. The target path must be absolute. A snapshot identifier is `latest` or 8 to 64 lower-case hexadecimal characters.
- The agent parses the restic `--json` output of a restore. The `status` messages become progress, and the `summary` message becomes a typed `RestoreSummary`. The job detail shows the summary. A dry run changes no file.

### Browsing

- A `snapshots` job lists the snapshots with the filters host, tag and path. The date filter is applied by the control plane when it serves the list. The control plane stores the list in `job_listings` after it bounded and cleaned each value that the agent sent. `GET /api/v1/repositories/{id}/snapshots` returns the list of a job, or the newest stored list with its age.
- An `ls` job reads one directory level. The agent runs `restic ls` for the directory, skips the first `offset` entries and keeps at most `limit` entries (the default is 1000, the maximum is 5000). The listed directory is not an entry of itself. When the page is full and more entries follow, the agent stops restic. A result therefore holds one page only. The first page of a directory with one million files comes in less than 5 s (AC-RS-02). The root is the path `/`, because `restic ls` without a directory lists the whole tree.
- The control plane answers a repeated page request from the stored copy for 10 minutes and starts no job. The sweep deletes the page listings after one hour. It deletes the snapshot listings and the finished browse jobs after seven days. The job list hides the browse jobs.
- A browse job holds no slot of its agent and no limit of its repository. A person waits for it, and a long backup must not hold the screen. It still follows the barrier of an exclusive job (FR-SC-04). The index of the active jobs of a repository excludes the kinds `snapshots`, `ls` and `restore`, like `backup`.
- A browse job expires after two minutes in the queue. An offline agent gives HTTP 409 `agent_offline` at once.

## Consequences

- The data of a restore goes from the repository to the target agent. The control plane sees the progress, the summary and the log only.
- The size of a step result stays below the limit of the stream for any directory (AC-RS-02).
- A new agent package is needed for the restore. An older agent keeps working for backups and for the snapshot list.
- The table `job_listings` can grow. The sweep bounds it by age.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| One `ls` step that sends the whole tree in many messages | The control plane would store or relay one million entries, and the first page would wait for the full tree. |
| A cursor token for the pages | The agent stays without state between pages, so an offset is enough. A page of a directory is cheap, because `restic ls` of one directory reads one tree. |
| Relay the restored data through the control plane | It breaks ADR-0003 and AC-RS-03. |
| Rely on the old agent to reject the new step kinds | A half-understood job must never run. The capability makes the refusal explicit and gives the user a reason. |
