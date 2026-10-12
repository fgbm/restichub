# ADR-0003: Direct repository access with secrets for one job

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-AG-03, FR-AG-06, FR-PR-05, FR-RP-08, NFR-SEC-08

## Context

restic has no key permissions. All keys of a repository open the same master key. A client with any key can read all data. On a backend with write and delete access, the client can also delete all data. Only rest-server in append-only mode stops a client from deleting data.

Possible models:

1. The agent accesses the repository directly.
2. A proxy holds the storage credentials. Kopia uses this model.
3. Append-only rest-server with a separate trusted host for maintenance.

## Decision

The agent accesses the repository directly. The control plane sends the repository password and backend credentials with each job. The agent gives them to restic through environment variables or a pipe. The agent does not store them in any form.

Only the maintenance agent of a repository runs `forget`, `prune` and `check` (FR-RP-08).

## Consequences

- No proxy component. Data goes directly from the host to the storage.
- Secrets are on the host only during a job.
- A compromised agent can read and delete the repository during a job. We accept this risk. The threat model documents it.
- The documentation recommends controls on the storage side (NFR-SEC-08). Examples: bucket versioning, lifecycle rules, separate credentials for each host group. Another control is a secondary copy that agents cannot access.
- An append-only rest-server stays possible as an option of the repository configuration.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Storage proxy in the control plane | All backup traffic goes through the control plane. More load and more complexity. |
| Append-only rest-server as the only model | It excludes direct S3 access, which most users need. |
| Secrets stored on the agent | A stolen host discloses the secrets at any time, not only during a job. |
