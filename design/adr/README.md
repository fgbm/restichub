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
| [0034](0034-repository-maintenance-schedule.md) | Repository maintenance schedule and retention isolation | accepted |
| [0035](0035-restore-and-snapshot-browsing.md) | Restore and snapshot browsing | accepted |
| [0036](0036-alerts-and-notification-channels.md) | Alerts, notification channels and the recovery point objective | accepted |
| [0037](0037-server-scripts-on-trusted-hosts.md) | Server scripts on trusted hosts | accepted |
| [0038](0038-agent-downloads-restic.md) | The agent downloads restic | accepted |
| [0039](0039-stall-watchdog-instead-of-job-time-limit.md) | A stall watchdog instead of a fixed job time limit | accepted |
| [0040](0040-remove-allow-list-hooks.md) | Remove the allow-list hooks | accepted |
| [0041](0041-re-registration-and-duplicate-detection.md) | Re-registration that keeps the agent ID, and duplicate detection | accepted |
