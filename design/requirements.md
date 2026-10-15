# ResticHub requirements

Status: accepted for the MVP. Date: 2026-10-12.

Priorities:

- **M**: MVP.
- **1.0**: first stable release.
- **L**: later than 1.0.

Each requirement has an ID. Acceptance criteria refer to these IDs. See [acceptance-criteria.md](acceptance-criteria.md).

## 0. Accepted decisions

| ID | Decision | Effect on the requirements | ADR |
|---|---|---|---|
| D1 | ResticHub is a separate open source product under the Apache License 2.0. The main repository is on GitHub. | Documentation, easy installation, signed releases, no telemetry by default, English and Russian user interface. | [0014](adr/0014-license-and-hosting.md) |
| D2 | Agents access repositories directly. The control plane sends secrets for the duration of one job. The agent does not store them in any form. | An agent that has a password during a job can delete the repository. ResticHub accepts this risk, documents it and decreases it with storage-side controls (NFR-SEC-08). | [0003](adr/0003-direct-repository-access.md) |
| D3 | High availability is not in the MVP. | The MVP is one control plane instance and PostgreSQL. The design must not block high availability later. | [0018](adr/0018-high-availability-deferred.md) |
| D4 | ResticHub replaces other file-level backup tools. | Windows and VSS are primary use cases. Application consistency comes from allow-listed hooks. Native database steps are out of scope (see section 5). | [0016](adr/0016-native-database-steps-deferred.md) |
| D5 | Hooks run only from an allow-list on the agent. | The control plane cannot change the allow-list. The allow-list is the only local configuration that the agent keeps on purpose. | [0004](adr/0004-hook-allow-list.md) |
| D6 | Multi-tenancy is mandatory from the first release. | Each object belongs to a tenant. The database schema and the protocol contain the tenant from the first migration. | [0006](adr/0006-multi-tenancy.md) |
| D7 | One certificate authority and one job signing key serve all tenants. | The agent certificate contains the tenant ID. | [0006](adr/0006-multi-tenancy.md) |
| D8 | If a scheduled time falls in a DST gap, the job runs once, immediately after the clock change. | Scheduler behavior is deterministic and documented. | [0010](adr/0010-queue-and-scheduler-on-postgresql.md) |

## 1. Purpose and scope

**Purpose.** Central management of restic file-level backups on a fleet of Windows, Linux and macOS hosts. The control plane (CP) makes all decisions about when jobs run. The agent only executes commands.

**Components:**

- Control plane: Rust, PostgreSQL, REST API with Server-Sent Events, web UI (Svelte 5).
- Agent: Rust, operating system service, runs restic.

**Glossary:**

| Term | Meaning |
|---|---|
| Tenant | An organization: the isolation unit that owns hosts, repositories, plans, profiles, secrets and memberships (ADR-0006, ADR-0024, ADR-0025). |
| Repository | A restic repository: backend address, password, backend credentials. |
| Profile | A declarative backup definition, similar to a resticprofile profile. |
| Plan | A profile assigned to agents, with a schedule. |
| Job | One run of a plan. |
| Step | One atomic command for an agent in a job. |
| Maintenance agent | The agent that runs `forget`, `prune` and `check` for a repository. |
| Hook | An allow-listed command on the agent that runs before or after restic. |

**Out of scope:**

- Bare-metal recovery and hypervisor-level virtual machine backup.
- Native database backup steps (MSSQL, PostgreSQL and others). Use hooks.
- Own deduplication, encryption or storage format. restic does these tasks.
- Mount and FUSE.

## 2. Roles

| Role | Scope | Permissions |
|---|---|---|
| Platform Admin | Installation | Creates, suspends and deletes tenants. Sets quotas. Reads the platform audit log. |
| Admin | Tenant | All tenant operations, including users, agent revocation and installation. |
| Operator | Tenant | Profiles, plans, manual runs, cancellation, restore. |
| Viewer | Tenant | Read only, including the tenant audit log. |

A user can be a member of more than one tenant. In the MVP a role applies to the full tenant. In 1.0 a role can apply to agent groups and repositories.

## 3. Functional requirements

### 3.1. Agent

| ID | Requirement | Priority |
|---|---|---|
| FR-AG-01 | The agent runs as an operating system service: Windows service as LocalSystem, systemd unit, launchd daemon. | M (Windows, Linux), 1.0 (macOS) |
| FR-AG-02 | The agent starts no action itself. The only exceptions are the connection to the CP, the heartbeat and the delivery of results. | M |
| FR-AG-03 | The agent does not store schedules, profiles, repository secrets or job history. | M |
| FR-AG-04 | The agent stores only the CP address, its private key and certificate, the CP root certificate and the public job signing key. It also stores the hook allow-list and an outbox of results that it did not send yet. The outbox contains no secrets. It also stores a job journal with accepted `job_id` values and their results (ADR-0020). The journal contains no secrets. | M |
| FR-AG-05 | The agent executes steps in the specified sequence. A step has no fixed wall-clock limit. The agent stops a step that makes no progress for the stall timeout (default 60 minutes). A profile can set an optional maximum duration for a job (ADR-0039). | M |
| FR-AG-06 | The agent starts restic as a child process. Secrets go to the child process only through environment variables or a pipe. Secrets never go into command-line arguments. | M |
| FR-AG-07 | The agent erases secrets from memory after each step. The agent disables crash dumps during a job where the operating system permits it. | 1.0 |
| FR-AG-08 | The agent parses the restic JSON output and streams progress and logs to the CP in real time. | M |
| FR-AG-09 | The agent maps restic exit codes to job statuses. 0: success. 3: warning. 11: retry (lock). 10 and 12: configuration error. 130: cancelled. All other codes: failed. | M |
| FR-AG-10 | The agent cancels a step in two stages. First it sends SIGINT on Unix or Ctrl+Break on Windows. After a grace period it kills the process tree. After a hard kill it reports "possible stale lock". | M |
| FR-AG-11 | The agent downloads restic itself and does not ship it in the agent package (ADR-0038). It checks the signed `SHA256SUMS` of the restic release and the archive hash before it unpacks, and it installs only a supported version. It records the SHA-256 of the installed binary in a protected file. At start the agent checks the hash and refuses a different binary. The control plane cannot set the restic path. The agent reports the agent, restic and operating system versions. ResticHub supports restic 0.18.1 and later. | M |
| FR-AG-12 | If the agent receives a `job_id` that it completed before, it does not execute the job again. It returns the stored result. | M |
| FR-AG-13 | Self-update accepts only a binary signed with the project release key, not with a CP key. After the update the agent does a health check. If it cannot connect in N minutes, it goes back to the previous binary. | 1.0 |
| FR-AG-14 | Resource limits: idle agent 30 MB RAM or less and 1 % CPU or less. During a job 100 MB RAM or less, without restic. | 1.0 |
| FR-AG-15 | The agent can limit restic bandwidth (`--limit-upload`, `--limit-download`) and set the process and I/O priority. | M |

### 3.2. Enrollment, identity, connection

| ID | Requirement | Priority |
|---|---|---|
| FR-ID-01 | Enrollment uses a one-time token with a short TTL. The token belongs to one tenant and can carry agent labels. | M |
| FR-ID-02 | The agent generates its key pair and sends a CSR. The CP signs the certificate with its CA. The certificate contains the agent ID and the tenant ID. | M |
| FR-ID-03 | The certificate has a limited lifetime (default 30 days). The agent rotates it through the existing mTLS channel at approximately 2/3 of the lifetime. | M |
| FR-ID-04 | When an Admin revokes an agent, the CP closes the stream immediately and refuses new connections from that agent. | M |
| FR-ID-05 | Enrollment is the only endpoint without mTLS. The CP limits its request rate and records each request in the audit log. | M |
| FR-ID-06 | A tenant can require that an Admin approves each new agent (quarantine mode). | 1.0 |
| FR-ID-07 | The agent keeps a permanent outbound bidirectional gRPC stream to the CP, with an application-level heartbeat. | M |
| FR-ID-08 | The agent reconnects with exponential backoff and jitter. After a CP restart, the CP waits for a grace period before it marks agents offline. | M |
| FR-ID-09 | Fallback transport for networks with restrictive proxies. | L |

### 3.3. Protocol and job model

| ID | Requirement | Priority |
|---|---|---|
| FR-PR-01 | Steps are typed: one Protocol Buffers message for each step type in a `oneof`. The protocol has no free-form maps or raw restic argument strings. | M |
| FR-PR-02 | Step types in the MVP: `backup`, `check`, `snapshots`, `ls`, `restore`, `unlock` (stale locks only), `unlock_all` (repository ID only, only from an Admin action with confirmation, FR-SC-06), `forget`, `prune`, `stats`, `hook`. In 1.0: `find`, `dump`, `copy`, `init`, `key_add`, `key_remove`. | M, 1.0 |
| FR-PR-03 | The CP signs each job with the job signing key (Ed25519). The signature covers `job_id`, `agent_id`, `tenant_id`, `issued_at` and `expires_at`. The signature input has the domain prefix `restichub-job-v1\0` before the serialized `Job` bytes. The agent rejects a job with a bad signature, a different addressee or a lifetime longer than 24 h. The agent also rejects a job with an `issued_at` more than 5 minutes in the future or an expired time. The agent keeps a job journal and returns the stored result of a known `job_id` (ADR-0020). | M |
| FR-PR-04 | The job signing key is separate from the TLS key and from the database. It is in a file in the MVP and in an HSM or KMS later. | M (file), L (KMS) |
| FR-PR-05 | Secrets go with the job, protected by mTLS. The agent gets only the secrets of the repositories in that job. | M |
| FR-PR-06 | The agent acknowledges each job. The CP holds a lease. If the lease expires before the acknowledgement, the CP delivers the job again. | M |
| FR-PR-07 | The agent keeps an outbox of results and sends them after it reconnects. | M |
| FR-PR-08 | The CP does not trust any message from an agent. The agent and tenant identity come only from the mTLS certificate, never from the payload. | M |
| FR-PR-09 | The protocol has a version. At connection time the agent and the CP agree on capabilities: agent version, restic version, supported step types. | M |
| FR-PR-10 | The agent puts all positional restic arguments after `--`. The agent passes each option value in the attached form `--flag=value`, never as a separate argument. The CP validates the formats before it signs the job and the agent validates them before execution (ADR-0021). A duplicate or unknown hook parameter name causes `INVALID_HOOK_PARAMETERS`. A step without a known kind causes `UNSUPPORTED_STEP`. The agent never sets an option or environment variable that executes a program from job data. | M |
| FR-PR-11 | A `KeyUpdate` message contains `key_id`, the public key, the signer key ID, `issued_at` and the revoked key IDs. The current key signs the message with the domain prefix `restichub-key-update-v1\0`. The agent never replaces an existing `key_id` and never adds a revoked key again. A revoked key cannot sign. | M |

### 3.4. Hooks

| ID | Requirement | Priority |
|---|---|---|
| FR-HK-01 | The allow-list is a file on the agent. Only root or Administrators can write it. The CP, self-update and jobs cannot change it. | M |
| FR-HK-02 | Each hook has: ID, executable, argument template, parameter schema (type, regular expression, enumeration), timeout, working directory, user account where the operating system permits it. | M |
| FR-HK-03 | The CP refers to a hook by ID and sends parameters. The agent validates the parameters against the schema and starts the hook without a shell. | M |
| FR-HK-04 | The agent reports the metadata of its allow-list to the CP. In a profile, the UI lets the user select only hooks that all target agents have. | M |
| FR-HK-05 | Hook phases: `before`, `after`, `on_success`, `on_error`, `finally`. | M |
| FR-HK-06 | The package contains example hooks: `pg_dump`, `mysqldump`, service stop and start. They are disabled by default. A host administrator enables them. | 1.0 |
| FR-HK-07 | A hook can send its output directly to restic without a temporary file (`backup --stdin-from-command`). | 1.0 |

### 3.5. Repositories and secrets

| ID | Requirement | Priority |
|---|---|---|
| FR-RP-01 | Create, read, update and delete repositories. Backends: S3-compatible, rest-server, SFTP, local or SMB path, B2, Azure, GCS, rclone. | M (S3, rest, local), 1.0 (SFTP, B2, Azure, GCS, rclone) |
| FR-RP-02 | Initialize a repository and test access from a selected agent. | M |
| FR-RP-03 | The CP encrypts secrets with AES-256-GCM. Envelope encryption: one data key for each tenant, encrypted by a key encryption key (KEK). The KEK is in a file or an environment variable outside the database. | M |
| FR-RP-04 | Pluggable KEK providers: HashiCorp Vault Transit, OpenBao, cloud KMS. | L |
| FR-RP-05 | After a user saves a secret, the UI and the API cannot show it. The user can only replace it. The CP records each delivery of a secret to an agent in the audit log. | M |
| FR-RP-06 | Rotation of the repository password through `key add` and `key remove` on the maintenance agent. Rotation of backend credentials. | 1.0 |
| FR-RP-07 | One restic key for each agent, so that agent revocation can also remove its key. | L |
| FR-RP-08 | Each repository has one maintenance agent. Only this agent runs `forget`, `prune` and `check`. A dedicated host is recommended. | M |
| FR-RP-09 | Capacity tracking: size, growth, bucket quota, alert near the limit. | 1.0 |

### 3.6. Profiles and compiler

| ID | Requirement | Priority |
|---|---|---|
| FR-PF-01 | A profile contains the repository, paths for each operating system, excludes, exclude files and tags. It also contains backup options (VSS, one file system, exclude caches, rate limits), job limits (stall timeout, maximum duration), hooks, retention and check options (`read-data-subset`). | M |
| FR-PF-02 | Inheritance with `extends`: deep merge, cycle detection, explicit merge rules for lists (replace or append). | 1.0 |
| FR-PF-03 | Variables: host name, operating system, agent name, agent labels, date. | M |
| FR-PF-04 | The compiler converts a profile to a sequence of steps. The UI shows a preview: the steps and the equivalent restic command line with masked secrets. | M |
| FR-PF-05 | Validation before save: references, hooks on the target agents, cron expressions, options against the operating system and the restic version. | M |
| FR-PF-06 | Profile revisions with diff and rollback. Each job refers to the revision that the compiler used. | 1.0 |
| FR-PF-07 | Import and export of profiles in YAML. Import of a subset of resticprofile configuration for migration. | 1.0 (YAML), L (resticprofile) |
| FR-PF-08 | Profile groups with sequential or parallel run. | 1.0 |

### 3.7. Scheduler and queue

| ID | Requirement | Priority |
|---|---|---|
| FR-SC-01 | Cron schedules with time zones and DST handling as in decision D8. The schedule belongs to the plan. | M |
| FR-SC-02 | Job creation is idempotent: unique key `(plan_id, fire_time)`. | M |
| FR-SC-03 | Persistent queue in PostgreSQL: `SKIP LOCKED`, lease, retries with exponential delay, dead-letter state. | M |
| FR-SC-04 | Serialization for each repository. Exclusive operations (`forget`, `prune`, `check`, `tag`, `rewrite`, `repair`, `key`) are barriers. While they run, no backup or read operation starts on that repository, and the opposite. More than one backup can run on one repository at the same time, up to a configurable limit. | M |
| FR-SC-05 | The CP always sends `--retry-lock`. Exit code 11 causes a delayed retry, not a failure. | M |
| FR-SC-06 | Stale lock recovery. The CP runs `unlock` without `--remove-all` only when it knows that no operation runs on the repository. The CP waits at least 30 minutes after it loses an agent that held a lock. Only an Admin can run the `unlock_all` step, manually and after a confirmation. | M |
| FR-SC-07 | Missed run policy for an offline agent: run all, run the latest only, skip. Each job has an expiry time. | M |
| FR-SC-08 | Concurrency limits for each agent, each repository and globally. Maintenance windows and blackout periods. | M (limits), 1.0 (windows) |
| FR-SC-09 | Manual run and cancellation from the UI. Cancellation on timeout. | M |
| FR-SC-10 | Event triggers: agent connection, webhook, chain "after plan A succeeds". | 1.0 (chain), L (webhook) |
| FR-SC-11 | Plan assignment to one agent or by a label selector (`os=windows, env=prod`). | M (agent), 1.0 (selector) |

### 3.8. Restore

| ID | Requirement | Priority |
|---|---|---|
| FR-RS-01 | Snapshot list with filters by host, tag, path and date. | M |
| FR-RS-02 | Snapshot file tree (`ls --json`) with paging. File search (`find`). | M (ls), 1.0 (find) |
| FR-RS-03 | Restore to the source agent or to a different agent. The target agent gets the repository secrets for the restore job only. Data does not go through the CP. The agent restores only below the restore roots that root or Administrators configure on the host, and never over its own files (ADR-0035). | M |
| FR-RS-04 | Restore options: target path, include and exclude, `--dry-run`, overwrite mode, `--verify`. | M |
| FR-RS-05 | Download of one file or an archive in the browser (`dump`, streamed through the CP) with a size limit. | 1.0 |
| FR-RS-06 | Scheduled test restore: sample files, restore to a temporary directory, compare, report "restore verified". | 1.0 |

### 3.9. Monitoring, alerts, integrations

| ID | Requirement | Priority |
|---|---|---|
| FR-MN-01 | Dashboard. For each agent and plan: time of the last successful backup, recovery point objective (RPO) status, lock status, errors, online status. | M |
| FR-MN-02 | Job history with logs and with statistics from the restic `summary`: files, size, added data, duration. | M |
| FR-MN-03 | Alerts: job failed, RPO violated, agent offline longer than N, repository not checked for N days, lock held longer than N, capacity. | M (first three), 1.0 (others) |
| FR-MN-04 | Notification channels: email, webhook, Telegram. | M (email, webhook), 1.0 (Telegram) |
| FR-MN-05 | Prometheus `/metrics` endpoint. | M |
| FR-MN-06 | Zabbix template: discovery of agents and plans, triggers. | 1.0 |
| FR-MN-07 | Analytics: repository growth, deduplication ratio, durations, linear capacity forecast. | 1.0 |
| FR-MN-08 | Scheduled reports in CSV and JSON. PDF reports. | 1.0, L (PDF) |

### 3.10. Job logs

| ID | Requirement | Priority |
|---|---|---|
| FR-LG-01 | Log storage is a separate component with backends `database` (default) and `s3`. The design is similar to remote logging in Apache Airflow. | M (database), 1.0 (s3) |
| FR-LG-02 | The connection to the log storage and the retention rules are separate settings. Retention is by age, by size or both, and can be different for each tenant. Retention works the same way for all backends. | M |
| FR-LG-03 | Job metadata (status, summary, duration) is always in the database. Only log text goes to the external storage. | M |
| FR-LG-04 | If S3 is not available, the CP writes logs to the database, sends an alert and does not fail the job. After a backend change, the CP can still read the old logs. | 1.0 |

### 3.11. Tenants

| ID | Requirement | Priority |
|---|---|---|
| FR-TN-01 | Each object belongs to a tenant: agents, repositories, secrets, profiles, plans, jobs, logs, audit records, tokens. | M |
| FR-TN-02 | The Platform Admin right is a right of the installation account, independent of the organizations. A Platform Admin manages tenants, creates an organization by name only, and manages the installation-wide accounts and their memberships. An organization without an active Admin has the derived state "no administrator". Admin, Operator and Viewer roles apply in one tenant. | M |
| FR-TN-03 | The enrollment token belongs to one tenant. Each agent belongs to exactly one tenant, and the certificate contains the tenant ID. | M |
| FR-TN-04 | Quotas for each tenant: agents, plans, concurrent jobs, log volume. | 1.0 |
| FR-TN-05 | Metrics and audit records have a tenant label. Each API token belongs to one tenant. | M |
| FR-TN-06 | The administrator can run the CP in single-tenant mode. In this mode the UI and the API do not show tenants. The administrator can switch to multi-tenant mode without data migration. | M |

### 3.12. Agent installation

| ID | Requirement | Priority |
|---|---|---|
| FR-IN-01 | The CP generates the ready agent registration command with the one-time enrollment token, for Windows and for Linux. The Web UI shows the command one time. | M |
| FR-IN-02 | Packages: MSI (GPO and Intune compatible), deb, rpm, pkg, container image of the agent. | 1.0 |
| FR-IN-03 | Remote installation over SSH: Linux, macOS, Windows with OpenSSH. | 1.0 |
| FR-IN-04 | Remote installation over WinRM: NTLM, Kerberos, HTTPS. Behind an abstraction with a fallback. | 1.0 (after spike P3) |
| FR-IN-05 | The CP does not store credentials for remote installation. It uses them once and records the operation in the audit log. | 1.0 |
| FR-IN-06 | Agent removal: stop the service, remove the files, revoke the certificate. | 1.0 |
| FR-IN-07 | The agent registers by hand. A person on the host unpacks the agent package and runs `restichub-agent register --url <control-plane-url> --token <token>`. The `--token` flag is the documented default. The options `--token-file <path>` and the standard input are optional, for a script. The token is single use with a short TTL and an Admin can revoke it. The agent generates its key pair, enrolls and stores its identity in the protected configuration directory. | M |
| FR-IN-08 | The agent installs itself as an operating system service. A person on the host runs `restichub-agent install-service`. The command writes and enables a Windows service or a systemd unit (launchd on macOS in 1.0). | M (Windows, Linux), 1.0 (macOS) |

FR-IN-03, FR-IN-04 and FR-IN-05 are out of the MVP. The MVP installs an agent by hand (ADR-0032).

### 3.13. User interface, API, audit

| ID | Requirement | Priority |
|---|---|---|
| FR-UI-01 | UI sections: Dashboard, Agents, Repositories, Profiles, Plans, Jobs (live logs), Restore, Settings, Audit. Analytics in 1.0. | M |
| FR-UI-02 | Real-time updates through Server-Sent Events. | M |
| FR-UI-03 | Profile editor: form and YAML with validation. | M (form), 1.0 (YAML) |
| FR-UI-04 | Dark and light themes. English and Russian languages. | M |
| FR-API-01 | Public REST API with an OpenAPI specification that the CP generates from code. API tokens with limited permissions. The API can do all operations that the UI can do. | M |
| FR-API-02 | Command-line client `restichubctl`. | L |
| FR-AU-01 | Audit log: sign-in, configuration changes, run, cancellation, restore, delivery of secrets to agents, agent enrollment and revocation, remote installation. Each record has user, time, IP address and diff. | M |
| FR-AU-02 | The audit log is append-only and protected by a hash chain. Export to syslog. | 1.0 |
| FR-AU-03 | Authentication: local users with password, TOTP, OIDC, LDAP or Active Directory. | M (local), 1.0 (TOTP, OIDC), L (LDAP) |
| FR-AU-04 | A user account is installation-wide. A user signs in with the username and the password only, without a tenant name, and switches between the tenants after the sign-in. An account with no membership signs in too and sees the platform screens or the empty state. | M |

## 4. Non-functional requirements

### 4.1. Security

| ID | Requirement |
|---|---|
| NFR-SEC-01 | mTLS (TLS 1.3, rustls) between the agent and the CP. HTTPS for the UI and the API. |
| NFR-SEC-02 | The threat model is in the repository. The maintainers review it before each release. |
| NFR-SEC-03 | A compromised CP cannot execute arbitrary code on agents, because of the allow-list and the separate job signing key. |
| NFR-SEC-04 | A compromised CP database without the KEK does not disclose secrets. Without the job signing key it does not let an attacker forge jobs. |
| NFR-SEC-05 | The agent does not write secrets to logs, files or process arguments. The agent masks secrets in restic and hook logs. |
| NFR-SEC-06 | Releases are signed (cosign or minisign, Authenticode for Windows, notarization for macOS). Builds are reproducible. Each release has an SBOM. |
| NFR-SEC-07 | CI checks dependencies with `cargo deny` and `npm audit`. |
| NFR-SEC-08 | The documentation describes the residual risk of D2 and the controls. The controls are bucket versioning, lifecycle rules, separate backend credentials for each host group and a dedicated maintenance agent. Another control is a secondary copy (`copy`) to a repository that agents cannot access. |
| NFR-SEC-09 | Responsible disclosure policy in `SECURITY.md`. |

### 4.2. Reliability

| ID | Requirement |
|---|---|
| NFR-REL-01 | A CP restart does not lose jobs or results. Jobs that run during the restart complete or run again. |
| NFR-REL-02 | If the connection breaks during a job, the agent completes the step and sends the result later. |
| NFR-REL-03 | The database schema has built-in migrations. A CP upgrade needs no manual steps. |
| NFR-REL-04 | The documentation tells how to back up and restore the CP: database, KEK, job signing key, CA. |
| NFR-REL-05 | The design permits high availability later. The CP keeps no state outside the database. |

### 4.3. Performance

| ID | Requirement |
|---|---|
| NFR-PERF-01 | One CP instance (2 vCPU, 4 GB RAM) serves 500 connected agents. Target for 1.0: 2000. |
| NFR-PERF-02 | From "Run" in the UI to the restic start on an online agent: 3 s or less. |
| NFR-PERF-03 | Progress shows in the UI 2 s or less after the agent sends it. |
| NFR-PERF-04 | A load test with emulated agents runs before each release. |

### 4.4. Compatibility

| ID | Requirement |
|---|---|
| NFR-CMP-01 | CP: Linux x86_64 and arm64. Container image and Docker Compose file. |
| NFR-CMP-02 | Agent in the MVP: Windows 10, 11 and Server 2016 or later (x86_64, arm64); Linux x86_64 and arm64 (static musl build, with or without systemd). Agent in 1.0: macOS 12 or later. |
| NFR-CMP-03 | restic 0.18.1 or later. Target version 0.19.x. A compatibility test suite checks each supported version. |
| NFR-CMP-04 | PostgreSQL 16 or later. |

### 4.5. Operations and open source

| ID | Requirement |
|---|---|
| NFR-OPS-01 | One command installs the CP (Docker Compose). A wizard does the initial setup. |
| NFR-OPS-02 | Structured JSON logs from the CP and the agent. Health and readiness endpoints. |
| NFR-OPS-03 | No telemetry by default. |
| NFR-OSS-01 | The repository contains LICENSE, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, ADR log and a public roadmap. |
| NFR-OSS-02 | Documentation: installation, threat model, migration from resticprofile, API reference. |
| NFR-OSS-03 | Semantic versioning. The CP supports agents of the current and the previous minor version. |

### 4.6. Documentation

| ID | Requirement |
|---|---|
| NFR-DOC-01 | English documentation obeys ASD-STE100 Simplified Technical English. See [style/ste-en.md](style/ste-en.md). |
| NFR-DOC-02 | Russian documentation obeys the project rules in [style/ru-technical.md](style/ru-technical.md). |
| NFR-DOC-03 | Vale checks the documentation in CI. The documentation build fails on errors. |
| NFR-DOC-04 | UI text, error messages and agent logs obey the same rules. The agent and the CP send error codes with parameters. The UI translates them. |

## 5. Deferred scope

| Item | Reason | ADR |
|---|---|---|
| Native MSSQL backup and restore steps (VDI streaming, LSN chains, point-in-time restore) | They increase the scope too much for the MVP and 1.0. | [0016](adr/0016-native-database-steps-deferred.md) |
| Native steps for other databases | Same reason. Hooks cover these cases. | [0016](adr/0016-native-database-steps-deferred.md) |
| High availability of the CP | Not necessary for the MVP. The design permits it. | [0018](adr/0018-high-availability-deferred.md) |
| ConnectRPC for the public API | connect-rust is before version 1.0. The maintainers evaluate it again for 1.0. | [0009](adr/0009-public-api-rest-openapi-sse.md) |
| SFTP repositories | Spike P6 found no SFTP key path without a file on the host that works on Linux and Windows. The rclone path adds about 86 MB and the Windows path is not verified. | [0022](adr/0022-sftp-deferred-to-1-0.md) |

## 6. Assumptions and risks

| Risk | Control |
|---|---|
| An agent with a password during a job can destroy the repository (D2). | NFR-SEC-08, dedicated maintenance agent, secondary copy. |
| Users can expect more than file-level backup. | Section 1 lists the items that are out of scope. |
| The Rust WinRM client is not mature. | Spike P3 before 1.0. Fallbacks: SSH, MSI with GPO. |
| Without high availability, no backups run while the CP is down. | Alert and documented recovery time for the CP. High availability is on the roadmap. |
| restic memory usage on large repositories. | Concurrency limits on the agent. Guidance on how to split repositories. |
| Ctrl+Break from a Windows service to restic can fail, because a service has no console. | Resolved by spike P1: mechanism B. |
| SFTP private key delivery without a file on the agent is not solved. | Deferred to 1.0 (ADR-0022). |
