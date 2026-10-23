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
| R | Release check: the release workflow or the release procedure runs it, and the report is part of the release |
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
| AC-AG-12 | FR-AG-05, FR-AG-15 | A backup step reports progress for longer than the stall timeout and for longer than 12 hours. **Expected:** the agent does not stop the step. A step whose status does not change for the stall timeout is stopped in two stages, and the job result has the code `stalled`. The verify phase of a `restore` prints nothing, and the agent does not see the files that it reads. The phase gets the longer of the stall timeout of the step and 24 hours. A phase without a line for that window stops in the same way. A backup with an upload limit of 50 KiB/s and a stall timeout of 5 minutes gets a raised stall timeout. **Expected:** the upload of a pack does not stop it. | A |
| AC-AG-13 | FR-AG-05 | A profile sets a maximum duration and the backup is still in progress after that time. **Expected:** the agent stops the step, and the job result has the code `max_duration_exceeded`. A profile without a maximum duration has no wall-clock limit. | A |
| AC-AG-14 | FR-AG-15 | A step asks for the low priority. **Expected:** on Linux restic starts with a nice value 10 above the agent (19 at most) and the lowest best-effort I/O class. On Windows restic starts in the below-normal priority class. A step without the low priority changes nothing. | A (Linux), M (Windows) |
| AC-AG-15 | FR-AG-04 | On Windows, a non-elevated administrator opens `C:\Program Files\ResticHub`, `C:\ProgramData\ResticHub` and `C:\ProgramData\ResticHub\Agent` in File Explorer and clicks "Continue". Restart the service. **Expected:** the service runs and connects. Each directory holds no entry of that account. Each child that inherited the entry holds no entry of that account. The `ResticHub` log holds a warning that names the path and the account. The warning for `Agent` tells that the identity was exposed to the account. When the administrator opens `Agent` while the service runs, the service starts again after the next break of the stream and removes the entry. The same entry for an account that is not an administrator stops the service with the exit code 78. Then `install-service` repairs the tree, or `register --force` replaces the identity (ADR-0048, ADR-0049). | S |

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
| AC-HK-12 | FR-HK-12 | An Operator edits a script or a template. **Expected:** the CP refuses it. An Admin edit writes an audit record with the hash of the script. A template of another organization returns 404. A Viewer reads a profile with empty script bodies and empty string variables, and a template returns 403. | A |
| AC-HK-13 | FR-HK-05, FR-HK-09 | A `before` script with the policy `abort_backup` fails. **Expected:** the backup does not run. `on_error` and `finally` scripts run. With the policy `continue`, the backup runs. | A |

## 6. Repositories and secrets

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-RP-01 | FR-RP-01, FR-RP-02 | Initialize and test S3, rest and local repositories (SFTP from 1.0). **Expected:** success. With an incorrect password: status `config_error` (code 12) and a clear message. | A |
| AC-RP-02 | FR-RP-03, FR-RP-05 | Read a secret through the API and the UI. **Expected:** the value is masked. No endpoint returns the value. A database dump does not contain the canary. The CP does not start with an incorrect KEK and shows the cause. | S |
| AC-RP-03 | FR-RP-05 | The CP sends a secret to an agent. **Expected:** the audit log has a record with tenant, agent, job, repository and time. The record does not contain the secret value. | A |
| AC-RP-04 | FR-RP-08 | A plan with retention runs on 5 agents for 7 days. **Expected:** only the maintenance agent receives `forget`, `prune` and `check`. The message log shows no such steps for the backup agents. | A |
| AC-RP-05 | FR-RP-10 | Create a local repository without a host, then with a host. **Expected:** the first request gives `400` on `maintenance_agent_id` with the reason `empty`. The second request creates the repository. A remote repository without a host is accepted. A drive path on a Linux host gives `400` on `backend.path` with the reason `path_on_other_os`. A path from `/` on a Windows host gives the same error. | A |
| AC-RP-06 | FR-RP-10 | Change the host of a local repository before and after the initialization. **Expected:** before the initialization the change is accepted when no plan of the repository runs on the old host, else it gives the reason `plan_on_other_host`. After the initialization it gives `409 repository_host_locked`. The host of an initialized remote repository changes. | A |
| AC-RP-07 | FR-RP-10 | On another host, save a plan of a local repository, and "Run now" such a plan. Change the repository of a plan to a local repository of another host. Start a restore, a snapshot list, a tree page or a test of the access, and queue a job. Repeat the plan save while a concurrent change moves the plan to such a repository. **Expected:** each request is refused with `not_repository_host` or `plan_on_other_host`. "Run now" gives `409 not_repository_host` without a field. The queued job fails with the result `config_error` and the reason `not_repository_host`, and the other host receives no job. | A |
| AC-RP-08 | FR-RP-10 | Replace the host record of an initialized local repository with a new record of the same machine. **Expected:** the repository host is the new record. | A |
| AC-RP-09 | FR-RP-10 | Start the CP with a local repository without a host and a plan on another host. **Expected:** the CP logs a warning for each and changes no row. The server does not wait for the check. | A |
| AC-RP-10 | FR-RP-11 | For each backend kind, create a second repository with the location of the first. Repeat with another spelling of the same location (host case, default port, trailing slash, `.` segment). Update a repository to the location of another one. Create a local repository with the same path on another host. **Expected:** each request of the same location gives `409 repository_location_taken` on the location field, with the name of the first repository. The request on another host, bucket, prefix or path is accepted. Another tenant can use the same location. | A |
| AC-RP-11 | FR-RP-11 | Start the CP with two repositories of one tenant that hold the same location. **Expected:** the CP logs a warning for each of them and changes no row. The list marks both repositories with the names of the others. The server does not wait for the check. | A |
| AC-RP-12 | FR-RP-01, FR-SC-01 | Delete a repository that one plan uses, then a repository that no plan uses. **Expected:** the first request gives `409 repository_in_use` with the list of the plans and changes nothing. The second request deletes the repository. A repository of another tenant gives HTTP 404. | A |
| AC-RP-13 | FR-RP-08, FR-PF-01 | Two profiles have their plans on one repository and on two hosts. A third profile has a plan in another repository only. **Expected:** the retention task of the repository has one `forget` step for each of the two profiles. Each step filters on the tag of its profile. Each step takes the agent facts from the plans of that profile in this repository. The third profile gets no step there. The check task reads the largest `read_data_subset` of the two profiles. | A |

## 7. Profiles

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-PF-01 | FR-PF-04 | Open the preview of a plan. **Expected:** the steps and the command line agree with the command that the agent ran (compare with the job log). Secrets and parameter values are masked. A Viewer sees the commands of the stored profile, with the script bodies and the string values hidden. | A |
| AC-PF-02 | FR-PF-05 | Save a profile with one error. Test each error: incorrect cron, a script without the trust mode, VSS for a Linux agent, option not available in the installed restic. **Expected:** the UI does not save the profile. The error points to the field. | A |
| AC-PF-03 | FR-PF-03 | A profile uses `{{hostname}}`, `{{os}}`, `{{agent.label.env}}` and the date on 3 agents. **Expected:** each agent gets its own values. | A |
| AC-PF-04 | FR-PF-02 (1.0) | A chain of `extends` with three levels. **Expected:** the result agrees with the documented merge rules (table-driven test). The CP rejects the cycle A→B→A and shows the chain. | A |
| AC-PF-05 | FR-PF-06 (1.0) | Change a profile and roll it back. **Expected:** the diff shows the change. The rollback restores the revision. Each job refers to the revision that the compiler used. | A |
| AC-PF-06 | FR-PF-11 | Save a plan whose profile has a required parameter and whose host has no value for it. Give the value on the host and save the plan again. **Expected:** the first request gives `parameter_missing` with the names of the parameters and changes nothing. The second request saves the plan. | A |
| AC-PF-07 | FR-PF-11 | Add a required parameter to a profile whose plans take the value from the host. Then clear a host value that a plan needs. Then give a value of the wrong kind. **Expected:** the first two requests give `parameter_missing_in_plans` with the list of the plans and change nothing. The third gives `parameter_kind_mismatch`. | A |
| AC-PF-08 | FR-PF-10 | A parameter has a default, the host has a value and a plan overrides the value. **Expected:** the preview and the job use the value of the plan. A plan without a value uses the value of the host. A host without a value uses the default. | A |
| AC-PF-09 | FR-PF-12 | Put a `secret` parameter into a path, an exclude, an exclude file and a tag. **Expected:** the CP refuses each save with a typed reason on the field. A `secret` value in a script variable reaches the agent as a secret value. | A |
| AC-PF-10 | FR-PF-12 | A host and a plan hold a `text` value with a canary. A script variable of the profile names that parameter. Read the host and the plan as a Viewer. Read the preview and the job as an Admin. Read the audit record. **Expected:** the Viewer sees the names of the parameters without the values. The preview shows the name of the variable and no canary. The job carries the value as a masked value, and the agent log shows no canary. The audit diff holds no canary. The audit holds the names and the identifiers of the secrets. A canary that the stored profile puts into a path or a tag is in the job command and in the snapshot. The control plane does not hide it there. | S |

## 8. Scheduler and queue

| ID | Requirement | Criterion | Method |
|---|---|---|---|
| AC-SC-01 | FR-SC-01 | Plan `0 2 * * *` in Europe/Moscow. **Expected:** the job starts at 02:00 Moscow time. Plan `30 2 * * *` in Europe/Berlin on the spring and autumn DST change days, with a simulated clock. **Expected:** exactly one run on each day. In the spring gap, the run occurs immediately after the clock change (D8). | A |
| AC-SC-02 | FR-SC-02 | Two CP processes on one database, 100 plans, 24 h of simulated time. **Expected:** exactly one job for each `(plan_id, fire_time)`. | A |
| AC-SC-03 | FR-SC-04 | 3 backups run on repository R. A prune for R goes into the queue. **Expected:** the prune starts after all 3 backups end. Backups queued during the prune start after the prune ends. | A |
| AC-SC-04 | FR-SC-04, FR-SC-05 | 7-day run: 20 agents, one repository, backup every 15 minutes, `forget` and `prune` every 2 hours, `check` every day. **Expected:** 0 jobs with status `failed` because of a lock. 0 errors from `restic check`. | L |
| AC-SC-05 | FR-SC-05 | Fake restic returns code 11 five times. **Expected:** retries with increasing delay. After the limit, the job goes to the dead-letter state and an alert occurs. | A |
| AC-SC-06 | FR-SC-06 | Kill restic with SIGKILL during a backup. **Expected:** when 30 minutes or more pass and the repository is idle, the CP runs `unlock`. The next backup succeeds without a person. Only an Admin can run the `unlock_all` step after a confirmation. The audit log contains the event. | A |
| AC-SC-07 | FR-SC-07 | An agent is offline for 3 plan intervals. **Expected:** the policies "run all", "run latest" and "skip" leave 3, 1 and 1 jobs in the queue. A fire time within the grace window of the pass is on time, and the control plane creates its job under every policy. The policies "run latest" and "skip" also apply while the agent stays offline. A new scheduled job replaces the older queued scheduled jobs of the plan that the agent did not take. They get the status `expired`. After a downtime of the control plane, the policy "run latest" creates one job for the latest older missed fire time. The policy "skip" creates none for the older missed fire times. A scheduled job of the policy "skip" has the same expiry time as the other scheduled jobs. | A |
| AC-SC-08 | FR-SC-08 | Limit 1 job for each agent and 2 jobs for each repository. Start 10 jobs at the same time. **Expected:** the start log shows that no limit was exceeded. | A |
| AC-SC-09 | FR-SC-09, NFR-PERF-02 | Manual run for an online agent. **Expected:** from the click to the restic start, 3 s or less (95th percentile of 100 runs). | L |
| AC-SC-10 | FR-SC-06 | A job runs on an online agent for longer than 12 hours. **Expected:** the CP keeps the job in the status `running`. When the agent stays away for 30 minutes, or connects and does not list the job, the CP fails the job with the reason `agent_lost`. A running job past the hard ceiling fails with the reason `timed_out`. | A |
| AC-SC-11 | FR-SC-11 | Create a plan for agent A, and change it to agent B. **Expected:** the plan stores the agent. The audit log records each change. The plan creates its jobs only for that agent. A plan for an unknown agent is refused. | A |
| AC-SC-12 | FR-SC-12 | The database refuses the job insert of one plan. Another plan has an invalid cron expression or an unknown time zone. **Expected:** each plan stays enabled and shows the failure reason and the first failure time in the API and in the UI. For the database error the fire time stays. The pause grows from 5 s to at most 10 min. After the recovery, the missed run policy decides which jobs the CP creates. For the other reasons the CP does not try again. It waits for an edit of the cron expression, the time zone or the enabled state. A plan of a creator whose membership is removed still advances, and the API token of this creator keeps working. With hundreds of failing plans, a healthy plan runs in the first pass after its fire time. The RPO alert of the failing plan stays open. The audit log records `schedule_failed` for the actor system when the failure starts, and `schedule_recovered` when a successful pass ends it. The same holds for a maintenance schedule. | A |

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
| AC-MN-10 | FR-MN-03, FR-AG-10 | A job runs and the service of the agent stops. **Expected:** the agent reports the result `cancelled`, and the job ends as cancelled and raises no `job_failed` alert, for a job of any kind. A backup with a script is delivered again because the CP did not get the acknowledgement, and the agent reports the code `interrupted`. **Expected:** the backup ends as cancelled with the reason `interrupted` and raises no `job_failed` alert. When the RPO of the plan passes without a new success, the RPO alert opens. An agent that reports `interrupted` for a job of another kind, for example a prune, gets a failed job and the alert. A job that the CP lost (`agent_lost`) and a restic error also end as failed and raise the alert. | A |
| AC-LG-01 | FR-LG-01, FR-LG-02 | Backend `database`, global retention of 7 days, simulated clock. **Expected:** logs older than 7 days are deleted. Job metadata stays. The setting is one value for all tenants and counts only the age. | A |
| AC-LG-02 | FR-LG-01, FR-LG-02 (1.0) | Change the backend to `s3`. **Expected:** new logs go to S3 with the keys `<tenant>/<job>/<step>`. The UI shows old logs from the database and new logs from S3. Retention in S3 works independently. | A |
| AC-LG-03 | FR-LG-04 (1.0) | MinIO is not available for 30 minutes. **Expected:** jobs do not fail. The logs go to the database. An alert occurs. | A |
| AC-LG-04 | NFR-SEC-05 | After the full end-to-end suite, search for the canary in all logs of all backends. **Expected:** 0 matches. | S |
| AC-LG-05 | FR-LG-07, FR-PR-07 | The stream drops in the middle of a backup that writes a long log. The agent restarts once, and the control plane acknowledges only a part of the log. The spool of the agent is full for a short time. **Expected:** after the reconnect, the CP has every line once and in order. The acknowledged data leaves the disk of the agent. A full spool leaves one marker line with the number of lost lines. A stalled CP does not delay a cancel. | A |
| AC-LG-06 | FR-LG-08 (1.0) | Set a retention size, and a different retention for two tenants. **Expected:** the control plane deletes the oldest log text when a tenant exceeds its size. Each tenant keeps its logs for its own period. | A |

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
| AC-UI-04 | FR-UI-05 | Set a tab, a filter, a sort and a page size on the Jobs list. Open another page from the navigation, then open Jobs from the navigation. **Expected:** the list shows the same tab, filter, sort and page size, and the address holds them. A reload gives the same result. | A |
| AC-UI-05 | FR-UI-05 | Open a list with a link that has parameters. **Expected:** the list uses the parameters of the link and not the kept ones. The kept view does not change until the person changes the list. | A |
| AC-UI-06 | FR-UI-05 | Use "Reset" in the filter bar of a list. **Expected:** the kept filters of that list are cleared. | A |
| AC-UI-07 | FR-UI-05 | Keep a filter value that no longer exists (a deleted host, repository or plan, or an unknown tab). **Expected:** the list drops that value without an error and applies the other kept values. | A |
| AC-UI-08 | FR-UI-05 | Sign in as a second user, or in a second tenant, in the same browser. **Expected:** the second person does not see the kept view of the first person. | A |
| AC-UI-09 | FR-UI-05 | On the Restore page, choose a repository, open another page, then return. Reload the page. **Expected:** the same repository is selected each time. This includes a repository that the page chose by itself on the first visit. | A |
| AC-UI-10 | FR-UI-05 | Close a section of the profile, plan, repository or host editor. Open another record of the same kind. **Expected:** the section is closed there too. The state is kept for each kind of editor and not for each record. | A |
| AC-UI-11 | FR-UI-05 | Choose a host in the restic commands panel of the profile editor, then open the editor again. **Expected:** the panel shows the same host. The choice is kept for the browser. | A |
| AC-UI-12 | FR-UI-05 | Fill the search field and go to page 2. Set the restore target, the overwrite mode, the dry run flag, the verify flag and a form field. Open the page again. **Expected:** none of these values is kept. The browser storage holds no secret. | A |
| AC-UI-13 | FR-UI-05 | Open the UI in a browser with no storage (private mode or disabled storage). **Expected:** the UI works and shows the default view. | A |
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
| AC-NFR-01 | NFR-PERF-01, NFR-PERF-04 | Bench S2: 500 agents, each gets a job every 15 minutes, for 4 hours. **Expected:** CP CPU below 70 % (95th percentile), RAM below 3 GB, dispatch delay below 3 s (95th percentile). | L, R |
| AC-NFR-02 | NFR-REL-01 | Kill the CP with SIGKILL while 50 jobs run. **Expected:** after the restart all jobs get a final status. No job is lost or runs two times. | A |
| AC-NFR-03 | NFR-SEC-06, NFR-SEC-07 | Make a release. **Expected:** all artifacts are signed with cosign. Two builds of the Linux agent package from one commit give the same archive bytes. The command in the documentation verifies the signature. The release has an SBOM. `cargo deny` and `npm audit` show no high or critical findings. | A |
| AC-NFR-04 | NFR-SEC-02, NFR-SEC-08 | Read the threat model. **Expected:** it is in the repository. It describes the residual risk of direct repository access and the controls from NFR-SEC-08. | M |
| AC-NFR-05 | NFR-OPS-01 | A person who did not write the documentation installs the product with it. **Expected:** from an empty virtual machine to the first successful backup in 30 minutes or less. | M |
| AC-NFR-06 | NFR-OPS-03 | Capture the network traffic of the CP and an agent for 24 h. **Expected:** no connections other than the configured ones. | S |
| AC-NFR-07 | NFR-CMP-01 | Make a release. **Expected:** the control plane image has the platforms `linux/amd64` and `linux/arm64`. The image starts with `compose.yaml`. | R |
| AC-NFR-08 | NFR-CMP-02 | Make a release. **Expected:** the agent packages exist for Linux (x86_64, arm64; static musl build) and for Windows (x86_64 only). The install smoke workflow registers the agent as a service on Linux with systemd and on Windows. Windows 10, Windows Server 2016 and Linux without systemd stay manual checks. | A, M |
| AC-NFR-09 | NFR-CMP-03 | Run the agent with restic 0.18.1 and with 0.19.1 on bench S1. **Expected:** backup, restore and the output parser work with both versions. An agent with an older restic gets no job (AC-AG-09). | R |
| AC-NFR-10 | NFR-CMP-04 | Run the database tests of CI. **Expected:** they use PostgreSQL 16 and pass. | A |
| AC-NFR-11 | NFR-OPS-02 | Start the control plane with `RESTICHUB_LOG_FORMAT=json`. **Expected:** each log line is a JSON object. `/healthz` answers `200`. `/readyz` answers `200` with a database. `/readyz` answers `503` while a background task of the CP is failing. | A |
| AC-NFR-12 | NFR-OSS-01 | List the root of the repository. Open the milestones of the repository on GitHub. **Expected:** the root holds LICENSE, CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md and `design/adr/`. The open milestones name their scope, for example MVP and 1.0. The README or CONTRIBUTING.md links to the milestones. | R |
| AC-NFR-13 | NFR-OSS-02 | Open the published documentation. **Expected:** it holds the installation guide, the threat model, the resticprofile migration guide and the API reference. | M |
| AC-NFR-14 | NFR-OSS-03 | Connect an agent of version N, one of version N-1 and one of the next major version. **Expected:** the control plane accepts the first two and refuses the last one with a typed reason (AC-PR-06). The release tag is a strict semantic version, and the release workflow refuses any other tag. | A, R |
| AC-NFR-15 | NFR-REL-03 | Upgrade the control plane image of a Compose installation and run `docker compose up`. **Expected:** the `migrate` service applies the migrations before `cp` starts, with no other step. The control plane refuses to start on a database with a newer schema and shows the cause. | A, R |
| AC-NFR-16 | NFR-REL-04 | Follow the backup guide on a running control plane. Restore it on a new host from the database dump, the KEK, the job signing key and the CA. **Expected:** the new control plane starts. The agents connect and a backup job runs. | M |
| AC-NFR-17 | NFR-REL-05 | Review the control plane code and run two instances on one database (AC-SC-02). **Expected:** the control plane keeps no state outside the database. A restart of an instance loses nothing (AC-NFR-02). | M |
| AC-NFR-18 | NFR-SEC-10 (1.0) | Make a release. **Expected:** `restichub-agent.exe` has a valid Authenticode signature. Two builds of the Windows package and of the images from one commit give the same bytes. | A |
| AC-NFR-19 | NFR-CMP-05 (1.0) | Install the Windows arm64 agent on a Windows 11 arm64 host. **Expected:** it registers, runs as a service and completes a backup. | A, M |
| AC-NFR-20 | NFR-OPS-04 (1.0) | Start the agent with the JSON log format. **Expected:** each log line is a JSON object. The standard error, the journald entry and the service log file hold the same objects. | A |
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
| AC-IN-04 | FR-IN-01 | In the Web UI, add a host with labels. **Expected:** the screen shows the enrollment token and the command `restichub-agent register --url <url> --token <token>` one time. The URL is the address of the control plane. The same command works for Windows and Linux (AC-IN-02). | A |
