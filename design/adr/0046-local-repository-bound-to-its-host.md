# ADR-0046: A local repository is bound to its host

- Status: accepted
- Date: 2026-10-17
- Requirements: FR-RP-01, FR-RP-08, FR-RP-10, AC-RP-05, AC-RP-06, AC-RP-07, AC-RP-08, AC-RP-09
- Related: ADR-0035, ADR-0041

## Context

A local repository is a path on the disk of one host. Restic on another host does not see this path, or it sees a different directory with the same name. Before this decision, the control plane accepted a local repository without a host and a plan of the repository on any host. A backup on another host then wrote to an empty directory or failed. The maintenance host (FR-RP-08) ran `forget`, `prune` and `check` on its own disk, which is possibly not the disk of the backups. ADR-0035 already sends a restore from a local repository to the maintenance host only.

## Decision

- A local repository has one host, the repository host. The repository host is the maintenance host of the repository. The control plane refuses a local repository without a host (`400`, field `maintenance_agent_id`, reason `empty`).
- Every job of a local repository runs on the repository host. This includes the backups of the plans, `forget`, `prune`, `check` and `unlock`. It also includes the initialization, the test of the access, the snapshot list, the tree pages and the restore.
- The control plane refuses a plan of a local repository on another host. It refuses a profile change to a local repository when a plan of the profile runs on another host. It refuses "Run now" of such a plan with `409 not_repository_host`, because the request has no agent field. The job builder refuses a queued job on another host with the reason `not_repository_host` and the result `config_error`. A retry cannot correct a configuration error.
- Before the initialization, the repository host can change. The control plane refuses the change while a plan of the repository runs on the old host.
- After the initialization, the repository host cannot change (`409 repository_host_locked`). The snapshots are on the disk of this host.
- The replace of a host record (ADR-0041) moves the repository host to the new record. The two records have the same machine identifier, so the disk and the path stay the same.
- The plan save locks the profile (share lock), then the agent that the write binds (key share lock), then the repository. The profile update locks the profile, then the repository. The replace locks the agents, then the plans, then the repositories. This order prevents a deadlock, and a concurrent profile change cannot pass the host check with the old repository.
- A revoked repository host gets no special procedure. The control plane does not move the repository to another host and does not free the host. The user does one of these steps:
  - Delete the repository in ResticHub. The delete removes the record and its secrets only. The restic data on the disk stays. If the data moves to a new host, add it as a new repository on the new host and run the test of the access.
  - Create a new local repository with the same path on the new host and a new profile that uses it. Move the plans to the new profile and the new host. Run the test of the access: it finds the existing repository and marks it initialized. Then delete the old repository.
- A shared network disk (NFS, SMB) that several hosts mount and use as a local repository is not supported. Use rest-server or SFTP for a repository that several hosts write to.
- The control plane changes no stored row at the upgrade. At the start, a background task logs a warning for each local repository without a host and for each plan on another host. The warning holds at most 20 plan identifiers for each repository.

## Consequences

- A backup, a restore or a maintenance job of a local repository always runs where the data is.
- Some installations used a shared network disk as a local repository from several hosts. After the upgrade, each plan that is not on the repository host gets failed jobs. The user moves such a repository to rest-server or SFTP.
- A host that is revoked before a replace leaves its initialized local repositories without a working host. The user follows the steps above. The job history and the maintenance schedules of the old repository record do not move to the new record.
- The user finds a plan on another host in the UI. Its jobs fail with the reason `not_repository_host`, "Run now" shows the error, and the plan page shows the error on the host field.
