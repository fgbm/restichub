# ResticHub acceptance criteria

Status: accepted for the MVP. Date: 2026-10-12.

This document refers to the requirement IDs in [requirements.md](requirements.md). Criteria for 1.0 requirements have the mark "(1.0)".

## 1. General rules

### 1.1. Definition of done

A requirement is done when all these conditions are true:

1. All its criteria pass on the reference test bench.
2. CI runs the automated tests (method A).
3. The acceptance report contains the results of the manual checks (method M).
4. The documentation obeys NFR-DOC-01 and NFR-DOC-02.

Each automated test refers to its criterion ID in its name or in a comment.

### 1.2. Verification methods

| Method | Meaning |
|---|---|
| A | Automated test |
| L | Load test |
| M | Manual check with a checklist |
| S | Security test |

### 1.3. Test benches

**Reference bench S1:**

- CP: Docker Compose, PostgreSQL 16.
- Agents: Windows Server 2022 x64, Windows 11 x64, Ubuntu 24.04 x64, Debian 12 arm64.
- Repositories: MinIO (S3), rest-server 0.14, local directory (SFTP from 1.0).
- restic versions: 0.18.1 and 0.19.1.

**Load bench S2:**

- CP on a virtual machine with 2 vCPU and 4 GB RAM.
- Emulator of 500 agents with a fake restic binary.

**Canary secret.** In all end-to-end tests, passwords and keys contain a unique marker, for example `CANARY-<uuid>`. The test searches for the marker in all locations where a secret must not be.

**Fake restic.** A test binary that writes recorded restic JSON output and exits with a specified code. It can also ignore signals.

## 2. Agent

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-AG-01 | FR-AG-01 | Install the agent on each operating system of S1. **Expected:** the service runs. It starts automatically after a reboot. After the process stops unexpectedly, the service starts again in 30 s or less. | A |
| AC-AG-02 | FR-AG-02, FR-AG-03 | The agent is connected to the CP and gets no jobs for 24 h. **Expected:** no restic process and no script starts. The agent opens outbound connections only to the CP. | A |
| AC-AG-03 | FR-AG-03, FR-AG-04 | The agent completes 10 jobs on 3 repositories. **Expected:** a search for the canary in the agent directory, temporary directories, logs, Windows registry, journald and Windows Event Log finds 0 matches. The agent files agree with the list in FR-AG-04. | S |
| AC-AG-04 | FR-AG-06 | During a backup, read the process list: `/proc/*/cmdline` on Linux, `Win32_Process.CommandLine` on Windows. **Expected:** no process argument contains the canary. | S |
| AC-AG-05 | FR-AG-09 | Fake restic exits with codes 0, 1, 3, 10, 11, 12, 130 and 99. **Expected:** statuses `success`, `failed`, `warning`, `config_error`, `retry`, `config_error`, `cancelled`, `failed`. | A |
| AC-AG-06 | FR-AG-10 | A backup runs. The operator clicks "Cancel". **Expected:** restic stops before the end of the grace period (default 30 s). `restic list locks` shows no locks. Status `cancelled`. Do this test on Linux and on Windows. | A |
| AC-AG-07 | FR-AG-10 | Fake restic ignores the signal. **Expected:** after the grace period the agent kills the process tree. Status `cancelled` with the flag `possible_stale_lock`. | A |
| AC-AG-08 | FR-AG-12 | The agent completed a job. The CP sends the same `job_id` again. **Expected:** restic does not start. The agent returns the stored result. | A |
| AC-AG-09 | FR-AG-11 | The agent has restic 0.18.1. The profile has an option that needs 0.19. **Expected:** the CP does not send the job. Status `incompatible`, with the option name and the necessary version. | A |
| AC-AG-10 | FR-AG-15 | Upload limit 10 MB/s. **Expected:** the average upload rate over 60 s is 11 MB/s or less. | A |
| AC-AG-11 | FR-AG-11 | Replace the installed restic binary with a different binary. **Expected:** the agent refuses to run restic and reports the mismatch. The control plane has no setting for the restic path. | S |
| AC-AG-12 | FR-AG-05 | A backup step reports progress for longer than the stall timeout and for longer than 12 hours. **Expected:** the agent does not stop the step. A step whose status does not change for the stall timeout is stopped in two stages, and the job result has the code `stalled`. | A |
| AC-AG-13 | FR-AG-05 | A profile sets a maximum duration and the backup is still in progress after that time. **Expected:** the agent stops the step, and the job result has the code `max_duration_exceeded`. A profile without a maximum duration has no wall-clock limit. | A |

## 3. Enrollment, identity, connection

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-ID-01 | FR-ID-01, FR-TN-03 | Enroll an agent with a valid token of tenant A. **Expected:** the agent shows only in tenant A. Its certificate contains `agent_id` and `tenant_id`. The CP rejects a used token and an expired token with a clear error. | A |
| AC-ID-02 | FR-ID-02 | After enrollment, examine a dump of the CP database. **Expected:** the dump contains no agent private keys. | S |
| AC-ID-03 | FR-ID-03 | Test certificate TTL 60 minutes. **Expected:** the agent rotates the certificate between minute 35 and minute 45. An active job continues. | A |
| AC-ID-04 | FR-ID-04 | An Admin revokes an agent during a job. **Expected:** the CP sends a cancellation and closes the stream in 5 s or less. The CP rejects the next connection. The audit log contains the event. | A |
| AC-ID-05 | FR-ID-05 | Scan the CP ports and endpoints without a client certificate. **Expected:** only enrollment, health and the authenticated UI and API are available. The 11th enrollment request from one IP address in one minute gets HTTP 429. | S |
| AC-ID-06 | FR-ID-08 | Bench S2: restart the CP while 500 agents are connected. **Expected:** all agents connect again in 5 minutes or less. No "agent offline" alert occurs during the grace period. | L |
| AC-ID-07 | FR-ID-10 | An Admin issues a token for a host with plans and registers a new certificate with it. **Expected:** the agent ID, the plans and the labels stay. The old certificate is refused. A second unused token of the same host is refused after the first use, and a token for a revoked host is refused and audited. | A |
| AC-ID-08 | FR-ID-10 | An Admin issues a token for a host. **Expected:** the time to live is one hour at most. A new token for the same host revokes the earlier unused token. A host that waits for its first connection after the registration raises no "host unreachable" alert. | A |
| AC-ID-09 | FR-ID-11 | Two hosts report the same machine identifier. **Expected:** both show the warning, and only the newer record offers the replacement. The replacement in the other direction is refused. The replacement of a record that was online in the online window is refused. | A |
| AC-ID-10 | FR-ID-11, FR-ID-04 | An Admin replaces an old record that has a running job and a queued job. **Expected:** the running job is cancelled as in AC-ID-04, the queued job moves to the new record, the audit log has `agent_revoke` and `agent_replace`. The hash of the machine identifier never shows the raw value. | A |

## 4. Protocol and job signatures

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-PR-01 | FR-PR-03 | Send jobs with these faults: a changed payload, a different `agent_id`, a different `tenant_id`, an expired job and an unknown key. Also send a job with an `issued_at` more than 5 minutes in the future and a job with a lifetime longer than 24 h. Also deliver a known `job_id` again after an agent restart. **Expected:** the agent rejects all the faulty jobs. No process starts for them. The CP receives the reason. The audit log contains the event. For the known `job_id`, the job does not run a second time and the agent returns the stored result. | S |
| AC-PR-02 | FR-PR-04 | Replace the job signing key in the CP configuration. Keep the database and TLS. **Expected:** agents reject all new jobs. A database dump does not contain the job signing key. | S |
| AC-PR-03 | FR-PR-05 | A job uses repository A. Examine the received message with a debug build of the agent. **Expected:** the message contains only the secrets of repository A. | S |
| AC-PR-04 | FR-PR-06 | The agent disconnects before the acknowledgement. **Expected:** after the lease expires, the CP delivers the job again. The job runs exactly once. | A |
| AC-PR-05 | FR-PR-08 | Agent 1 sends a result or progress for a job of agent 2. **Expected:** the CP rejects the message and records it in the audit log. The status of the job of agent 2 does not change. | S |
| AC-PR-06 | FR-PR-09 | Connect agents of version N−1 minor, N and the next major version. **Expected:** the first two work. The CP rejects the third with an incompatibility message. | A |
| AC-PR-07 | FR-PR-07 | The CP is not available for 60 minutes during a job. **Expected:** the agent completes the job. The result and the logs arrive 60 s or less after the connection comes back. The final status and summary agree with the local data. | A |
| AC-PR-08 | FR-PR-10 | Put `--password-command=cmd /c calc`, `-x` and a line break into each string field of each step type. **Expected:** validation rejects the value, or the argument builder puts it after `--` or in attached form. restic never receives the value as an option. | S |
| AC-PR-09 | FR-PR-11 | Send a key update signed by a revoked key, a key update that changes an existing `key_id` and a replayed old key update. **Expected:** the agent rejects all three. The key ring does not change. | S |

## 5. Scripts

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-HK-02 | FR-HK-01 | Do a self-update and 100 jobs of different types. **Expected:** the checksum of the trust mode file does not change. | A |
| AC-HK-07 | FR-HK-08, FR-HK-10 | The trust mode is off and a job has a script step. **Expected:** the CP refuses the job with a typed reason. A job that reaches the agent anyway gets `JobRejected`. No script runs. | A |
| AC-HK-08 | FR-HK-08 | Another user can write the trust file. **Expected:** the agent reports the mode as off and runs no script. | A |
| AC-HK-09 | FR-HK-09 | A script has a secret parameter with a canary value. **Expected:** the script reads the canary from its environment. The canary is not in the arguments, the logs, the journal or a file after the run. The file with the body does not exist after the run. | S |
| AC-HK-10 | FR-HK-09 | A script runs longer than its timeout, or the user cancels the job. **Expected:** the agent stops the script and its child processes. | A |
| AC-HK-11 | FR-HK-11 | A job has scripts and a backup. **Expected:** the planned steps list all of them in order before they run. Each step has its own log. The API returns the lines of one step. | A |
| AC-HK-12 | FR-HK-12 | An Operator edits a script or a template. **Expected:** the CP refuses it. An Admin edit writes an audit record with the hash of the script. A template of another organization returns 404. | A |
| AC-HK-13 | FR-HK-05, FR-HK-09 | A `before` script with the policy `abort_backup` fails. **Expected:** the backup does not run. `on_error` and `finally` scripts run. With the policy `continue`, the backup runs. | A |

## 6. Repositories and secrets

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-RP-01 | FR-RP-01, FR-RP-02 | Initialize and test S3, rest and local repositories (SFTP from 1.0). **Expected:** success. With an incorrect password: status `config_error` (code 12) and a clear message. | A |
| AC-RP-02 | FR-RP-03, FR-RP-05 | Read a secret through the API and the UI. **Expected:** the value is masked. No endpoint returns the value. A database dump does not contain the canary. The CP does not start with an incorrect KEK and shows the cause. | S |
| AC-RP-03 | FR-RP-05 | The CP sends a secret to an agent. **Expected:** the audit log has a record with tenant, agent, job, repository and time. The record does not contain the secret value. | A |
| AC-RP-04 | FR-RP-08 | A plan with retention runs on 5 agents for 7 days. **Expected:** only the maintenance agent receives `forget`, `prune` and `check`. The message log shows no such steps for the backup agents. | A |

## 7. Profiles

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-PF-01 | FR-PF-04 | Open the profile preview. **Expected:** the steps and the command line agree with the command that the agent ran (compare with the job log). Secrets are masked. | A |
| AC-PF-02 | FR-PF-05 | Save a profile with one error. Test each error: incorrect cron, a script without the trust mode, VSS for a Linux agent, option not available in the installed restic. **Expected:** the UI does not save the profile. The error points to the field. | A |
| AC-PF-03 | FR-PF-03 | A profile uses `{{hostname}}`, `{{os}}`, `{{agent.label.env}}` and the date on 3 agents. **Expected:** each agent gets its own values. | A |
| AC-PF-04 | FR-PF-02 (1.0) | A chain of `extends` with three levels. **Expected:** the result agrees with the documented merge rules (table-driven test). The CP rejects the cycle A→B→A and shows the chain. | A |
| AC-PF-05 | FR-PF-06 (1.0) | Change a profile and roll it back. **Expected:** the diff shows the change. The rollback restores the revision. Each job refers to the revision that the compiler used. | A |

## 8. Scheduler and queue

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-SC-01 | FR-SC-01 | Plan `0 2 * * *` in Europe/Moscow. **Expected:** the job starts at 02:00 Moscow time. Plan `30 2 * * *` in Europe/Berlin on the spring and autumn DST change days, with a simulated clock. **Expected:** exactly one run on each day. In the spring gap, the run occurs immediately after the clock change (D8). | A |
| AC-SC-02 | FR-SC-02 | Two CP processes on one database, 100 plans, 24 h of simulated time. **Expected:** exactly one job for each `(plan_id, fire_time)`. | A |
| AC-SC-03 | FR-SC-04 | 3 backups run on repository R. A prune for R goes into the queue. **Expected:** the prune starts after all 3 backups end. Backups queued during the prune start after the prune ends. | A |
| AC-SC-04 | FR-SC-04, FR-SC-05 | 7-day run: 20 agents, one repository, backup every 15 minutes, `forget` and `prune` every 2 hours, `check` every day. **Expected:** 0 jobs with status `failed` because of a lock. 0 errors from `restic check`. | L |
| AC-SC-05 | FR-SC-05 | Fake restic returns code 11 five times. **Expected:** retries with increasing delay. After the limit, the job goes to the dead-letter state and an alert occurs. | A |
| AC-SC-06 | FR-SC-06 | Kill restic with SIGKILL during a backup. **Expected:** when 30 minutes or more pass and the repository is idle, the CP runs `unlock`. The next backup succeeds without a person. Only an Admin can run the `unlock_all` step after a confirmation. The audit log contains the event. | A |
| AC-SC-07 | FR-SC-07 | An agent is offline for 3 plan intervals. **Expected:** the policies "run all", "run latest" and "skip" create 3, 1 and 0 jobs. The count is the missed fire times. A fire time within the grace window of the pass is on time, and it is in addition to the count. Expired jobs get the status `expired`. | A |
| AC-SC-08 | FR-SC-08 | Limit 1 job for each agent and 2 jobs for each repository. Start 10 jobs at the same time. **Expected:** the start log shows that no limit was exceeded. | A |
| AC-SC-09 | FR-SC-09, NFR-PERF-02 | Manual run for an online agent. **Expected:** from the click to the restic start, 3 s or less (95th percentile of 100 runs). | L |
| AC-SC-10 | FR-SC-06 | A job runs on an online agent for longer than 12 hours. **Expected:** the CP keeps the job in the status `running`. When the agent stays away for 30 minutes, or connects and does not list the job, the CP fails the job with the reason `agent_lost`. A running job past the hard ceiling fails with the reason `timed_out`. | A |

## 9. Restore

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-RS-01 | FR-RS-01 | Filter snapshots by host, tag, path and date. **Expected:** the list agrees with `restic snapshots --json` with the same filters. | A |
| AC-RS-02 | FR-RS-02 | A snapshot with 1 million files. **Expected:** the first page of the tree shows in the UI in 5 s or less. Next pages load on request. | L |
| AC-RS-03 | FR-RS-03 | Restore 10 GB from agent A to agent B. **Expected:** the SHA-256 of all files agrees. Traffic through the CP is less than 1 % of the data volume. After the job, agent B contains no canary. | A |
| AC-RS-04 | FR-RS-04 | Restore with `--dry-run`. **Expected:** the file system does not change. Include, exclude and overwrite modes give the results in the table-driven test. | A |
| AC-RS-05 | FR-RS-03 | Restore to `/`, to `/etc`, to the configuration directory of the agent, to a link that points to it, and to a path outside the restore roots. **Expected:** the agent rejects each job with `RESTORE_TARGET_DENIED`, the control plane refuses a target outside the reported roots before a job exists, and no file changes. A path inside the default root works. | A |

## 10. Monitoring and logs

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-MN-01 | FR-MN-01, FR-MN-03 | Plan RPO 24 h. No successful backup for 24 h. **Expected:** an alert by email and webhook 5 minutes or less after the violation. After a successful backup, a recovery notification. | A |
| AC-MN-02 | FR-MN-03 | A job fails. **Expected:** an alert with tenant, agent, plan, error text and a link to the job. An agent offline longer than N causes a separate alert. | A |
| AC-MN-03 | FR-MN-05, FR-TN-05 | `/metrics` passes `promtool check metrics`. **Expected:** the metrics `last_success_timestamp`, `job_duration_seconds`, `agent_up` and `repo_lock_held` are present. Each metric has the label `tenant`. | A |
| AC-MN-04 | FR-MN-02 | Open the statistics of a job in the UI. **Expected:** the values agree with the restic `summary`. | A |
| AC-MN-05 | FR-MN-04 | A webhook URL names a loopback, private, link-local or metadata address, or a host name that resolves to one. **Expected:** the control plane refuses the URL or the send, and the target sees no request. An address in the installation allow-list passes. | A |
| AC-MN-06 | FR-MN-04 | A webhook receives an alert. **Expected:** the request holds the headers `X-ResticHub-Timestamp` and `X-ResticHub-Signature`. The signature verifies with the secret of the channel. | A |
| AC-MN-07 | FR-MN-03, FR-MN-04 | A condition stays open, and two control plane instances run. **Expected:** each message is sent once, with no repeat while the alert is open. A failed send retries with a backoff and stops after five attempts. | A |
| AC-MN-08 | FR-MN-04, NFR-SEC-04 | A webhook channel is created, rotated and listed. **Expected:** the secret appears in the create response and in the rotation response only. No read, audit record or database row holds it in clear text. | A |
| AC-MN-09 | FR-MN-01 | A tenant holds hosts, plans, repositories, jobs and alerts. **Expected:** the dashboard shows the hosts online and offline. It shows each plan with its last success and RPO state. It shows the repository lock state, the recent failed jobs and the open alerts. It shows rows of that tenant only. | A |
| AC-LG-01 | FR-LG-01, FR-LG-02 | Backend `database`, retention 7 days, simulated clock. **Expected:** logs older than 7 days are deleted. Job metadata stays. | A |
| AC-LG-02 | FR-LG-01, FR-LG-02 (1.0) | Change the backend to `s3`. **Expected:** new logs go to S3 with the keys `<tenant>/<job>/<step>`. The UI shows old logs from the database and new logs from S3. Retention in S3 works independently. | A |
| AC-LG-03 | FR-LG-04 (1.0) | MinIO is not available for 30 minutes. **Expected:** jobs do not fail. The logs go to the database. An alert occurs. | A |
| AC-LG-04 | NFR-SEC-05 | After the full end-to-end suite, search for the canary in all logs of all backends. **Expected:** 0 matches. | S |

## 11. Tenants

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-TN-01 | FR-TN-01 | An automated test calls each API endpoint as a user of tenant A. It uses object IDs of tenant B in the calls. **Expected:** HTTP 404 everywhere and no changes. The test fails if the OpenAPI specification has an endpoint without an isolation check. | S |
| AC-TN-02 | FR-TN-03 | An agent of tenant A sends a forged `tenant_id` in the payload. **Expected:** the agent does not receive jobs or secrets of tenant B. | S |
| AC-TN-03 | FR-TN-02 | The Platform Admin suspends a tenant. **Expected:** the CP dispatches no jobs. Users cannot sign in. Agents stay connected. The platform audit log contains the event. | A |
| AC-TN-04 | FR-TN-05 | Use an API token of tenant A. **Expected:** no access to objects of tenant B. The audit log of tenant A has no events of tenant B. | A |
| AC-TN-05 | FR-TN-04 (1.0) | Quota of 10 agents. **Expected:** the CP rejects the 11th enrollment with a clear error. | A |
| AC-TN-06 | FR-TN-06 | Start the CP in single-tenant mode and complete the setup. **Expected:** sign-in has no tenant field, platform endpoints return 404, all objects belong to tenant `default`. Switch to multi-tenant mode and restart. **Expected:** the CP issues a setup token for the Platform Admin, and the data of `default` stays available. | A |
| AC-TN-07 | FR-TN-02 | The Platform Admin creates an organization with the name only. **Expected:** the organization is active, the user count is 0, the admin state is `none`, and the platform audit log holds `tenant_create` alone. | A |

## 12. User interface, API, roles, audit

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-UI-01 | FR-API-01 | Compare the UI calls with the OpenAPI specification. **Expected:** each UI operation has an endpoint in the specification. Coverage 100 %. | A |
| AC-UI-02 | FR-UI-02 | Watch job progress in the UI. **Expected:** update delay 2 s or less (95th percentile). | L |
| AC-UI-03 | FR-UI-04 | Change the language to English and to Russian. **Expected:** the i18n linter finds no text without a translation. | A |
| AC-AU-01 | FR-AU-01 | Do each operation from the list in FR-AU-01. **Expected:** the audit log has a record with user, time, IP address and diff. The API has no endpoint that changes or deletes audit records. | A |
| AC-AU-02 | Section 2 | Table-driven test "role × operation". **Expected:** a Viewer gets HTTP 403 for each write operation. An Operator cannot manage users or revoke agents. | A |
| AC-AU-03 | FR-AU-04 | Sign in with the username and the password only. The account has one active membership. **Expected:** a session in that tenant. The sign-in screen has no tenant field. | A |
| AC-AU-04 | FR-AU-04 | Sign in with the username and the password only. The account has two active memberships. **Expected:** a session in the last used tenant, or in the first by name when the account has no last used tenant. `GET /api/v1/auth/tenants` lists the two memberships only. | A |
| AC-AU-05 | FR-AU-04 | Switch tenants with `POST /api/v1/auth/switch`. **Expected:** a new session in the target tenant, the old session is gone, the cookie rotates. The audit log holds `session_switch` in both tenants. A tenant without membership gives HTTP 403. | A |
| AC-AU-06 | FR-AU-04 | Sign in when the account has no active membership (the memberships are absent, or their tenants are suspended or deleted). **Expected:** the sign-in succeeds with no current organization. A Platform Admin reaches the platform screens. Another account sees the empty state "no organizations, ask an administrator". | S |
| AC-AU-07 | FR-AU-04 | A tenant Admin resets the password or the display name of a shared account. It creates an account with a username that exists in another tenant. **Expected:** HTTP 403 `shared_account` for the first two calls, HTTP 409 `username_taken` for the create. | A |
| AC-AU-08 | FR-AU-04 | The Platform Admin creates an account and adds a membership in the first organization, then a membership in the second. **Expected:** the account signs in and reaches the first, and the switch to the second works (AC-AU-04). | A |
| AC-AU-09 | FR-TN-02 | The Platform Admin calls `GET /api/v1/platform/users`. **Expected:** the list holds every account of every organization with its memberships and roles. A tenant Admin gets HTTP 403. | A |
| AC-AU-10 | FR-AU-04 | Create an account with an email that another account holds. **Expected:** HTTP 409 `email_taken`. The control plane treats the email of a new account as unverified. | A |
| AC-AU-11 | FR-TN-02 | Complete the setup of a new installation. **Expected:** the setup creates one account with the Platform Admin right and no membership. The account signs in and reaches the platform screens. | A |
| AC-AU-12 | FR-AU-04 | Open the organization switcher as a Platform Admin with no membership. **Expected:** the switcher shows the empty state and the Platform administration entry. | A |
| AC-AU-13 | FR-AU-04 | A Platform Admin is a member of organization A and organization B. **Expected:** the switcher lists A and B only, and the switch works (AC-AU-04). | A |
| AC-AU-14 | FR-TN-02 | Open the platform screens (Organizations, Users, Audit) as a Platform Admin in any current organization. **Expected:** the screens open with no organization switch. | A |
| AC-AU-15 | FR-TN-02 | A tenant Admin calls a platform endpoint. **Expected:** HTTP 403. The Platform Admin right is necessary. | A |
| AC-AU-16 | FR-TN-02 | Remove the Platform Admin right from the last active Platform Admin, or from itself. **Expected:** HTTP 409 for the last one and HTTP 403 for the own account. | A |
| AC-AU-17 | FR-AU-04 | A Platform Admin holds the Viewer role in organization A and reads the tenant data of A. **Expected:** the answer agrees with the Viewer role. The right adds no access (ADR-0006). | A |
| AC-AU-18 | FR-TN-02 | Apply the migration to an installation with `platform_admin` memberships. **Expected:** every such account holds the right and the memberships are gone. | A |
| AC-AU-19 | FR-TN-02 | Grant the Platform Admin right to an existing account without a password, then with a new password. **Expected:** HTTP 400 for the first. After the grant the old password gives HTTP 401, the old sessions are gone, and the new password signs in. | A |
| AC-TN-08 | FR-TN-01 | List the organizations and open the switcher. **Expected:** the internal platform tenant appears in no user-facing list and is no membership target. | A |

## 13. Non-functional requirements

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-NFR-01 | NFR-PERF-01 | Bench S2: 500 agents, each gets a job every 15 minutes, for 4 hours. **Expected:** CP CPU below 70 % (95th percentile), RAM below 3 GB, dispatch delay below 3 s (95th percentile). | L |
| AC-NFR-02 | NFR-REL-01 | Kill the CP with SIGKILL while 50 jobs run. **Expected:** after the restart all jobs get a final status. No job is lost or runs two times. | A |
| AC-NFR-03 | NFR-SEC-06, NFR-SEC-07 | Make a release. **Expected:** all artifacts are signed. The command in the documentation verifies the signature. The release has an SBOM. `cargo deny` and `npm audit` show no high or critical findings. | A |
| AC-NFR-04 | NFR-SEC-02, NFR-SEC-08 | Read the threat model. **Expected:** it is in the repository. It describes the residual risk of direct repository access and the controls from NFR-SEC-08. | M |
| AC-NFR-05 | NFR-OPS-01 | A person who did not write the documentation installs the product with it. **Expected:** from an empty virtual machine to the first successful backup in 30 minutes or less. | M |
| AC-NFR-06 | NFR-OPS-03 | Capture the network traffic of the CP and an agent for 24 h. **Expected:** no connections other than the configured ones. | S |
| AC-DOC-01 | NFR-DOC-01, NFR-DOC-03 | Build the documentation in CI. **Expected:** Vale finds no errors. Procedural sentences have 20 words or fewer. Descriptive sentences have 25 words or fewer. | A |
| AC-DOC-02 | NFR-DOC-01, NFR-DOC-02 | Review each new section with the style checklist. **Expected:** one instruction in each step, imperative mood, one term for one concept, terms from the glossary. | M |
| AC-DOC-03 | NFR-DOC-04 | Take 50 error messages from the UI and the agent. **Expected:** each message tells what occurred and what to do. Each message passes Vale. | M |

## 14. MVP release acceptance

The MVP is accepted when all these conditions are true:

1. All criteria of requirements with priority M pass on bench S1. Load criteria pass on bench S2.
2. These end-to-end scenarios pass, each with a report:
   1. A new tenant: from creation to the first backup.
   2. A Windows file server: backup with VSS and restore to a different host.
   3. Automatic recovery after a stale lock.
   4. Agent revocation during a job.
3. The 7-day run of AC-SC-04 completes without lost jobs and without false alerts.
4. The installation guide, the threat model and the resticprofile migration guide are published and pass AC-DOC-01.

## 15. Agent installation and packaging

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-IN-01 | FR-AG-11, FR-IN-07 | Open the agent package. **Expected:** it contains `restichub-agent`, the license text, a third-party notice and the package hashes. It does not contain `restic`. After `register`, the installed `restic` has the SHA-256 from the signed restic `SHA256SUMS`. | A |
| AC-IN-02 | FR-ID-01, FR-IN-07 | Register a host with `restichub-agent register --url <url> --token <token>`. **Expected:** the host appears active with the certificate of ADR-0026. The control plane consumes the token. A second registration with the same token, an expired token and a token that an Admin revoked are each refused with a clear error. | A |
| AC-IN-03 | FR-IN-08 | Run `restichub-agent install-service` on Windows and on Linux. **Expected:** the operating system registers the service and it starts after a reboot. The workflow `Install smoke (AC-IN-03)` covers the registration, the start and the connection to a control plane on Linux and on Windows. The reboot stays a manual check. | A |
