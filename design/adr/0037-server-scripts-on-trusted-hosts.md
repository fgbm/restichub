# ADR-0037: Server scripts on trusted hosts

- Status: accepted
- Date: 2026-10-15
- Requirements: FR-HK-01 to FR-HK-12, NFR-SEC-03
- Supersedes: [ADR-0004](0004-hook-allow-list.md) for hosts with the local trust mode

## Context

The allow-list of [ADR-0004](0004-hook-allow-list.md) is safe, but it is hard to use. A host administrator must install each hook on each host. The owner needs more flexibility and more visibility. A job can have any number of scripts before and after a backup. A user writes them in the web UI. Each script has its own log and duration. Templates help the user.

A script that comes from the control plane (CP) is code that runs as SYSTEM or root. A CP compromise then gives code execution on every host that accepts such scripts. The risk is acceptable only for a host whose administrator accepts it on purpose.

## Decision

1. A host has a local trust mode, `server_scripts`. It is off by default. A host administrator turns it on with the elevated command `restichub-agent trust-server-scripts --enable`, and off with `--disable`. The setting is a file in the protected configuration directory. It has the same owner and permission checks as the allow-list file. If another user can write the file, the mode is off. A job, the CP and a self-update cannot change it.
2. The agent reports the mode and the interpreters it found in the `AgentHello` facts. The capability is `scripts-v1`.
3. A job can contain a typed `ScriptStep`: interpreter (`powershell`, `cmd`, `bash`, `sh`), script body (64 KiB at most), name, phase, order, timeout, failure policy (`abort_backup` or `continue`), and environment parameters. A value is a plain string or a reference to a secret that the CP resolves at send time ([ADR-0017](0017-secret-encryption.md)). The agent gives the values to the script only as environment variables.
4. The job signature covers the script body ([ADR-0005](0005-signed-jobs.md)). The agent writes the body to a new protected file in its cache directory. It starts the interpreter with fixed safe flags. It bounds and scrubs the output. It kills the process tree on timeout or cancel, and deletes the file.
5. The CP refuses a job with a script for an agent without the mode, with a typed reason. The agent refuses it too (`JobRejected`).
6. The allow-list hooks of ADR-0004 stay. They are the only mode for a host without the trust mode. A profile can use both kinds.
7. Each script and each hook is its own job step with a status, a start time, a duration, an exit code and its own log. The CP stores the step index with each log line.
8. Only an Admin can edit scripts and script templates. An Operator can run the jobs. Each change of a script or of a template writes an audit record with the SHA-256 hash of the content. Each run writes the hash in the job record.
9. The CP ships a set of read-only built-in script templates. A tenant has its own script templates and job templates.

## Consequences

- A CP or tenant Admin compromise gives code execution on the hosts with the trust mode. It gives none on other hosts (threat T33).
- The mitigations are the local opt-in and the signed jobs. Every change and run is in the audit log. Only an Admin edits. The content hash is in the job and in the audit.
- A host administrator decides for each host. The UI shows the mode and the command to turn it on.
- A script has the rights of the agent service. The documentation says so in the risk statement.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep only the allow-list | It does not give the flexibility and the visibility that the owner needs. |
| Scripts on all hosts without a local opt-in | One CP compromise gives code execution on all hosts. |
| Remote approval of each script | The approver and the signer are in the same system, so a compromise bypasses it. |
