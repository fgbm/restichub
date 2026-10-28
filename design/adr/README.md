# Architecture decision records

Each record tells one decision, its context and its consequences. Do not change an accepted record. To change a decision, write a new record and mark the old record "superseded".

To write a new record, copy [0000-template.md](0000-template.md) and use the next free number.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | accepted |
| [0002](0002-control-plane-owns-scheduling.md) | The control plane owns all scheduling | accepted |
| [0003](0003-direct-repository-access.md) | Direct repository access with secrets for one job | accepted |
| [0004](0004-hook-allow-list.md) | Hooks only from an agent-side allow-list | superseded by ADR-0040 |
| [0005](0005-signed-jobs.md) | Signed jobs with a dedicated signing key | partly superseded by ADR-0020 (replay protection) |
| [0006](0006-multi-tenancy.md) | Multi-tenancy from the first migration | accepted |
| [0007](0007-modular-monolith.md) | Modular monolith: one control plane binary and PostgreSQL | accepted |
| [0008](0008-agent-protocol.md) | Agent protocol: Protocol Buffers, gRPC, mTLS, own CA | accepted |
| [0009](0009-public-api-rest-openapi-sse.md) | Public API: REST, OpenAPI and Server-Sent Events | accepted |
| [0010](0010-queue-and-scheduler-on-postgresql.md) | Job queue and scheduler on PostgreSQL | accepted |
| [0011](0011-restic-cli-integration.md) | restic integration through the CLI and JSON output | accepted |
| [0012](0012-web-ui-svelte.md) | Web UI: Svelte 5 single-page application in the control plane binary | accepted |
| [0013](0013-documentation-starlight-ste.md) | Documentation: Starlight, STE, Vale | accepted |
| [0014](0014-license-and-hosting.md) | Apache License 2.0 and GitHub | accepted |
| [0015](0015-log-storage.md) | Pluggable job log storage | accepted |
| [0016](0016-native-database-steps-deferred.md) | Native database steps are deferred | accepted |
| [0017](0017-secret-encryption.md) | Envelope encryption of secrets with a pluggable KEK provider | accepted |
| [0018](0018-high-availability-deferred.md) | High availability is deferred | accepted |
| [0019](0019-yaml-library.md) | serde-saphyr for YAML profile import and export | accepted |
| [0020](0020-replay-protection-job-journal.md) | Replay protection with a job journal | accepted |
| [0021](0021-restic-argument-construction.md) | restic argument construction and validation | accepted |
| [0022](0022-sftp-deferred-to-1-0.md) | SFTP is deferred to 1.0 | accepted |
| [0023](0023-control-plane-core.md) | Control plane core: storage, authentication and secret format | partly superseded by ADR-0025 (installation-wide user accounts) and ADR-0028 (organization create without an administrator) |
| [0024](0024-single-tenant-mode.md) | Single-tenant mode | partly superseded by ADR-0025 (installation-wide user accounts) |
| [0025](0025-installation-wide-user-accounts.md) | Installation-wide user accounts | partly superseded by ADR-0029 (the Platform Admin is an account right) |
| [0026](0026-agent-pki-identity-and-key-rotation.md) | Agent PKI, identity and key rotation | accepted |
| [0027](0027-web-ui-component-primitives.md) | Web UI component primitives | accepted |
| [0028](0028-platform-user-management.md) | Organization creation without an administrator, and platform user management | accepted |
| [0029](0029-platform-admin-account-right.md) | The Platform Admin is an account right, not a membership | accepted |
| [0030](0030-job-signing-key-resync.md) | Re-sync of the job signing key set | accepted |
| [0031](0031-bundled-restic-in-agent-package.md) | Agent package with a bundled restic | accepted, bundling superseded by ADR-0038 |
| [0032](0032-manual-agent-registration-and-service-install.md) | Manual agent registration and service install | accepted |
| [0033](0033-enrollment-transport-and-agent-trust-bootstrap.md) | Enrollment transport and agent trust bootstrap | accepted |
| [0034](0034-repository-maintenance-schedule.md) | Repository maintenance schedule and retention isolation | accepted, partly amended by ADR-0047 (the plans of the repository) |
| [0035](0035-restore-and-snapshot-browsing.md) | Restore and snapshot browsing | accepted |
| [0036](0036-alerts-and-notification-channels.md) | Alerts, notification channels and the recovery point objective | accepted, the closing of a one-time job alert amended by ADR-0050, the reconnect grace of the absence tests amended by ADR-0052 |
| [0037](0037-server-scripts-on-trusted-hosts.md) | Server scripts on trusted hosts | accepted |
| [0038](0038-agent-downloads-restic.md) | The agent downloads restic | accepted |
| [0039](0039-stall-watchdog-instead-of-job-time-limit.md) | A stall watchdog instead of a fixed job time limit | accepted, the grace for a fresh job superseded by ADR-0043, the rule of progress of a backup and a restore refined by ADR-0051 |
| [0040](0040-remove-allow-list-hooks.md) | Remove the allow-list hooks | accepted |
| [0041](0041-re-registration-and-duplicate-detection.md) | Re-registration that keeps the agent ID, and duplicate detection | accepted |
| [0042](0042-agent-log-spool-and-acknowledgements.md) | The agent log spool and the log acknowledgements | accepted |
| [0043](0043-no-grace-for-unlisted-running-jobs.md) | No grace for a running job that the hello omits | accepted |
| [0044](0044-host-online-state.md) | The host online state is an open stream recorded in the database | accepted, the threshold of the alert "Host offline" amended by ADR-0052 |
| [0045](0045-repository-roots-for-local-repositories.md) | Repository roots for local repositories | accepted |
| [0046](0046-local-repository-bound-to-its-host.md) | A local repository is bound to its host | accepted, partly amended by ADR-0047 (the plan holds the repository) |
| [0047](0047-repository-in-the-plan-and-profile-parameters.md) | The repository in the plan, and profile parameters | accepted |
| [0048](0048-administrator-access-entries-on-windows.md) | Self-repair of administrator access entries on Windows | accepted, decision 3 superseded by ADR-0049 (the identity directory) |
| [0049](0049-heal-the-identity-directory-of-an-administrator.md) | Heal the administrator entry on the identity directory | accepted |
| [0050](0050-closing-of-a-one-time-job-alert.md) | The closing of a one-time job alert | accepted |
| [0051](0051-a-restic-retry-is-not-progress.md) | A retry of restic is not progress | accepted |
| [0052](0052-reconnect-grace-of-the-absence-tests.md) | The reconnect grace of the absence tests | accepted |
| [0053](0053-server-scripts-outside-the-agent-sandbox.md) | A server script runs in a transient systemd unit | accepted |
| [0054](0054-about-restichub-endpoint.md) | The About ResticHub endpoint and its role boundary | accepted, decision 5 superseded by ADR-0055 (the environment of the server and the disclosure of the answer) |
| [0055](0055-about-environment.md) | The environment of the server in the About ResticHub answer | accepted, supersedes decision 5 of ADR-0054 |
