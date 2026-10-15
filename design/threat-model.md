# Threat model

Status: draft. Date: 2026-10-12. The maintainers review this document before each release (NFR-SEC-02).

## 1. Assets

| Asset | Where it is |
|---|---|
| Backup data | restic repositories |
| Repository passwords and backend credentials | Control plane database, encrypted. Agent memory during a job. |
| Key encryption key (KEK) | File or environment variable on the control plane host |
| Job signing keys (current and next) | Control plane database, wrapped by the KEK (ADR-0026) |
| CA private keys (root and issuing) | Control plane database, wrapped by the KEK (ADR-0026) |
| Agent private keys | Each agent host |
| Hook allow-lists | Each agent host |
| Audit log | Control plane database |

## 2. Trust boundaries

1. Browser or API client and control plane.
2. Agent and control plane.
3. Agent and repository backend.
4. Control plane and PostgreSQL.
5. Tenant and tenant in one installation. The account rows are installation-wide (ADR-0025).

## 3. Threats and controls

| ID | Threat | Control | Residual risk |
|---|---|---|---|
| T1 | A compromised agent reads or deletes a repository during a job. | Secrets only for one job (ADR-0003). Dedicated maintenance agent. Storage controls in NFR-SEC-08. | **Accepted.** An attacker with root on the host during a job can delete the repository if the backend permits deletion. |
| T2 | A compromised control plane executes code on all hosts. | Hook allow-list on agents (ADR-0004). Server scripts only on hosts with the local trust mode (T33). Typed steps without raw arguments (FR-PR-01). A restore writes only below the restore roots of the host, and never on the files of the agent (ADR-0035, like ADR-0004). | The control plane can still send restic steps, for example `forget` with a short retention. It can restore any snapshot of a repository below a restore root of an agent. Storage controls in NFR-SEC-08 limit the damage. An administrator who sets the root `/` accepts an in-place restore of any path except the files of the agent. |
| T3 | An attacker writes to the database and inserts jobs. | Job signatures with a key outside the database (ADR-0005). | None if the signing key stays safe. |
| T4 | Replay of an old job. | Job journal and maximum lifetime (ADR-0020). | A redelivered job returns the stored result. It does not run again. |
| T5 | A database dump discloses secrets. | Envelope encryption with a KEK outside the database (ADR-0017, ADR-0023). | If an attacker gets the KEK and the dump, all secrets are disclosed. |
| T6 | One agent impersonates another agent or another tenant. | Identity only from the mTLS certificate (FR-PR-08). The agent pins the installation root CA from the enrollment response (ADR-0026, ADR-0033). Row-Level Security (ADR-0006, ADR-0023). | None. |
| T7 | A user of one tenant reads data of another tenant. | Application checks, Row-Level Security (ADR-0006, ADR-0023), isolation test AC-TN-01. | Defects in new endpoints. AC-TN-01 fails on unchecked endpoints. |
| T8 | A stolen enrollment token enrolls a false agent. | One-time token with a short TTL. Optional quarantine mode (FR-ID-06). The enrollment endpoint runs on the agent listener and on the public web origin (ADR-0033). | A false agent can receive jobs of its tenant until an Admin revokes it. A TLS-intercepting proxy on the public origin can replace the installation root CA in the enrollment response. The agent then pins the wrong root for the stream. A token bound to an existing host (FR-ID-10) takes over the identity of that host with its plans: it lives one hour at most, a new bound token or a successful use revokes the other unused bound tokens of the host, and a refused use is audited. The machine identifier (FR-ID-11) is a hint: the agent sends only a digest of it, the CP stores a keyed hash, and the replacement of a record needs an Admin action, the right direction and a silent old record (ADR-0041). |
| T9 | Secrets in process arguments, logs or crash dumps. | Environment variables or pipe only (FR-AG-06). Masking (NFR-SEC-05). Crash dumps disabled (FR-AG-07). | A local administrator can read the environment of a running process. |
| T10 | A malicious update of the agent. | Release key separate from control plane keys (FR-AG-13). | Compromise of the release key. |
| T11 | Denial of service through enrollment. | Rate limit (FR-ID-05). | A distributed attack. Use a reverse proxy with rate limits. |
| T12 | Option injection through step string fields. | Argument construction and validation (ADR-0021, AC-PR-08). | Unknown restic behavior in new step types. |
| T13 | A forged signing key update. | Key update rules (FR-PR-11, AC-PR-09). A signing key compromise retires the key at once, and the signed update revokes it on every open stream (ADR-0030). | The first public key needs a trusted delivery path. An agent that was more than one rotation behind during a compromise enrolls again (ADR-0030). |
| T14 | A tenant Admin takes over an account of another tenant. | The account row is installation-wide and separate from the membership (ADR-0025). A tenant Admin cannot reset the password or change the display name of a shared account (HTTP 403 `shared_account`). It cannot attach an existing foreign account or disable the account. | None for an account with no membership in another tenant. |
| T15 | Username enumeration through the account create endpoint. | The create endpoint needs the Admin role and a rate limit of 20 creates per minute for each actor and tenant. Invitations by email arrive in 1.0 (ADR-0025). | **Accepted.** The answer HTTP 409 `username_taken` reveals an existing name. A tenant Admin can probe names. |
| T16 | Session fixation on the tenant switch. | The switch deletes the old session and rotates the cookie. The new session token is 256 random bits (ADR-0023, ADR-0025). | None. |
| T17 | A revoked agent stays connected or reconnects. | The control plane checks the registry status and the certificate serial at each handshake and stream open (ADR-0026, FR-ID-04). A revoke cancels the running jobs and closes the stream of the instance that serves the revoke at once. Each other instance checks the registry rows of its own streams every 2 seconds and closes the stream of a revoked agent. | A stream on another instance stays open for up to 2 seconds plus the query time. A person who steals the current private key of an agent can rotate it into an identity of their own. The next connection of the real agent then shows a superseded serial. The control plane refuses it, writes the audit record `agent_superseded_certificate_used` and marks the host on the Hosts screen. The real agent is pushed out until an Admin revokes the agent and registers the host again (ADR-0026). |
| T18 | A CA private key or a job signing key in a database dump. | The KEK wraps each key outside the database (ADR-0017, ADR-0023, ADR-0026). | An attacker with the KEK and the dump gets the keys. `rcgen` and `ring` keep some private key bytes in the heap in a buffer that they do not zeroize. The control plane holds the plaintext only for the current issuing CA. It drops every other copy at load. It refuses a core dump on Linux (ADR-0026). |
| T19 | An attacker swaps a stored key blob. | The AAD binds the ciphertext to the tenant and the key role. The start check binds the key to the certificate of its row (ADR-0026). | None. |
| T20 | A tampered restic binary on a host. | The agent installs restic only after it verifies the OpenPGP signature of the release `SHA256SUMS` with the pinned release key and the archive hash. The binary lives in a protected directory. A protected record holds its SHA-256, and the agent refuses a different binary at each start and spawn. The control plane cannot set the restic path (ADR-0038). | **Accepted.** An attacker with administrator rights on the host can replace the binary and the protected record together. The check raises the cost of an accidental or a partial swap and stops a network attacker. |
| T21 | A token in the shell history or in a process argument. | The token is single use with a short TTL and an Admin can revoke it (FR-ID-01). A successful registration consumes it. The audit log records the create, the use and the revoke. | **Accepted.** The token stays in the shell history and in the process list until the registration ends. Before the first use, or after a failed registration until the TTL ends, the value is live. |
| T22 | A Platform Admin reads the tenant data of an organization through the platform right. | The right is an account attribute and gives no tenant data access (ADR-0029). A platform operation runs in the platform scope; the tenant data needs a membership and its role (ADR-0006, ADR-0025). AC-AU-17 checks it. | None for the right alone. The operator grants a membership on purpose. |
| T23 | A Platform Admin adds a membership for itself and hides the change. | The write needs the Platform Admin right, and it is an audit record in the platform audit and in the target organization's audit (ADR-0029). | A Platform Admin can add itself to any active organization. The operator reads the platform audit. |
| T24 | A user-facing list shows the internal platform tenant. | The organization list, the switcher and the tenant API exclude the platform tenant (ADR-0029). AC-TN-08 checks it. | None. |
| T25 | An organization Admin takes over an account that holds the Platform Admin right, also when the account is a member of that organization only. | A tenant-scope write on such an account is refused (ADR-0029): the password, the display name, the disable, the role change and the membership removal. The guard is in the administrator functions and in a trigger on `memberships`. AC-AU-07 checks it. | None. |
| T26 | A password that an organization Admin knows survives a grant of the Platform Admin right. | A grant of the right to an existing account requires a new password chosen by the Platform Admin. The grant ends every session of the account in the same transaction (ADR-0029). A must-change flag is not enough, because the Admin signs in first with the known password. AC-AU-19 checks it. | None. |
| T27 | A `secret_delivered` audit record for a job that the control plane refused to send. | The dispatcher checks that the agent is online before it reads a secret, so a refused send normally writes no record. The record and the job carry the same `job_id`, so an operator can match them (AC-RP-03). | **Accepted.** If the agent disconnects between the online check and the send, the record exists while the job is `failed`; the secret never left the control plane. |
| T28 | A tenant Admin sets a webhook URL that points to an internal service or to the cloud metadata address (SSRF). | The control plane resolves the host name, refuses every forbidden address and connects to the checked address itself. It follows no redirect and reads no response body. The check includes the IPv6 forms that wrap an IPv4 address, `2001::/23`, `64:ff9b:1::/48` and `192.88.99.0/24`. The lookup and the connection share one time limit. The test button has a rate limit for each actor and tenant, and a bound on the tests that run at once. The installation allow-list is the only way around the check (ADR-0036, AC-MN-05). | The allow-list applies to every tenant of the installation. The operator adds a network only for a trusted installation. |
| T29 | The signing secret of a webhook or the SMTP password leaks through the API, the audit log or a log. | The signing secret and the path and query of the webhook URL (which often hold a token) are encrypted with the tenant key. The signing secret is shown once, and the URL path never. The generic secret API refuses the purposes of these secrets. The audit diff redacts it. The SMTP password comes from a file or the environment and never enters the database (ADR-0017, AC-MN-08). | A person who copies the secret at creation holds it. |
| T30 | An alert floods a channel or the outbox. | The partial unique index allows one open alert for a key. The outbox holds at most 1000 pending messages for a tenant. A failed message stops after five attempts (AC-MN-07). | A tenant with many failing plans gets one message for each plan. |
| T31 | A stolen Operator token or a compromised control plane restores a snapshot over the files of an agent: its identity, journal, binary or system files. The agent runs as root or LocalSystem, so it can write there. | The agent restores only below the restore roots that root or Administrators configure in a protected file. It always refuses a target that overlaps its own files, after it resolves links. It reports the roots, and the control plane refuses early. ADR-0035 and ADR-0004 use the same rule. AC-RS-05 checks it. | An administrator who adds the root `/` accepts a restore to any other directory. A file of the agent, a parent of one and a path inside one stay refused. A restore to the target `/` is not possible. A restore below a root still replaces files there. The agent ignores a root that another account can change, and a root with a parent that another account can change. Such an account could replace a directory with a link. |
| T32 | A snapshot list or a tree page with an explicit `agent_id` sends the repository keys to that agent. An Operator can name any active agent of the organization. | The agent must be active, online and in the same organization. Each delivery writes `secret_delivered`, and the job writes `browse_start` with the repository and the agent (ADR-0035, ADR-0017). A request has a limit for each actor and for each agent. | **Accepted.** An Operator can already restore to any agent of the organization, so the same trust applies. The operator reads the audit log. |
| T33 | A compromised control plane or a compromised tenant Admin sends a script that runs as SYSTEM or root on a trusted host. | The trust mode is local, off by default, and no job can change it (ADR-0037, AC-HK-07, AC-HK-08). The job is signed, and the signature covers the script body. Only an Admin edits scripts and templates. Each change and each run is in the audit log with the SHA-256 hash of the script. The agent runs the script with fixed interpreter flags, bounds its output and masks secret values (AC-HK-09). | **Accepted** for a host whose administrator turns on the trust mode. On such a host the CP has code execution with the rights of the agent service. The documentation says so. |

## 3.1 Tests for the threats

NFR-SEC-02 asks for a review of the threat rows before each release. This table names the automated tests of each threat, so the review can run them. A threat marked "Manual" has no automated test in the repository: its control is a build or an operator procedure.

The tests of `restichub-cp/tests/threat_model_e2e.rs` run the real control plane, the real listener and the emulator or the real agent checks. Their names start with the threat IDs.

| Threat | Tests | Kind |
|---|---|---|
| T1 | `ac_rp_01_the_secret_never_reaches_the_job_row_or_the_audit` (a secret lives for one job only). The loss of a repository to a host with root during a job is accepted. | Automated, residual risk accepted |
| T2 | `ac_pr_08_injected_values_stay_out_of_option_positions`, `ac_rs_05_a_target_outside_the_roots_or_on_the_agent_files_is_rejected` | Automated |
| T3 | `t3_t13_a_forged_job_and_a_forged_key_update_are_refused_by_the_agent`, `ac_pr_01_rejects_a_bad_signature`, `ac_pr_01_forged_job_is_refused` | Automated |
| T4 | `t4_a_replayed_job_does_not_run_twice`, `ac_pr_01_a_replay_of_a_completed_job_returns_the_stored_result`, `ac_pr_01_rejects_an_expired_job` | Automated |
| T5 | `ac_rp_02_a_database_dump_does_not_contain_the_canary`, `ac_pr_02_a_database_dump_holds_no_job_signing_key` | Automated |
| T6 | `t6_an_agent_cannot_act_for_another_agent`, `a_wrong_tenant_san_is_refused`, `ac_tn_02_the_request_carries_no_tenant_and_the_subject_is_ignored` | Automated |
| T7 | `ac_tn_01_a_cross_tenant_identifier_returns_404_and_changes_nothing`, `ac_tn_01_an_agent_of_another_tenant_is_not_found`, `ac_tn_01_another_tenant_gets_404_on_the_queue_endpoints` | Automated |
| T8 | `ac_in_02_a_used_expired_revoked_or_unknown_token_is_refused`, `ac_id_01_concurrent_requests_with_one_token_create_one_agent`, `ac_id_01_a_used_or_expired_token_is_refused`, `ac_id_07_a_second_bound_token_dies_with_the_first_use`, `ac_id_08_a_bound_token_lives_one_hour_at_most`, `ac_id_09_the_replacement_runs_only_from_the_newer_record`, `ac_id_09_the_replacement_is_refused_while_the_old_record_is_online`, `ac_id_07_a_refused_bound_token_of_a_revoked_host_is_audited` | Automated |
| T9 | `t9_t21_the_enrollment_and_the_rotation_leave_no_key_or_token_in_the_database_or_the_logs`, `ac_ag_03_a_canary_in_the_agent_directory_is_found`, `ac_ag_04_a_canary_in_a_process_argument_is_found` | Automated |
| T10 | The release key and the signed update are a release procedure (FR-AG-13). | Manual |
| T11 | `ac_id_05_the_eleventh_enrollment_from_one_ip_is_rate_limited` | Automated |
| T12 | `ac_pr_08_injected_values_stay_out_of_option_positions`, `ac_pr_08_newline_and_nul_values_are_rejected`, `fr_pr_10_a_restore_value_never_becomes_an_option` | Automated |
| T13 | `t3_t13_a_forged_job_and_a_forged_key_update_are_refused_by_the_agent`, `ac_pr_09_rejects_bad_signature`, `ac_pr_09_rejects_replayed_update_of_revoked_key`, `adr_0030_a_compromise_rotates_and_retires_the_key_with_the_reason`, `adr_0030_a_compromise_on_one_instance_reaches_the_agents_of_another`, `adr_0030_a_redelivery_after_a_compromise_never_sends_the_old_blob`, `adr_0030_an_agent_one_rotation_behind_follows_the_chain_after_a_compromise` | Automated |
| T14 | `ac_au_02_an_admin_cannot_grant_the_platform_admin_role`, `db_a_shared_account_counts_a_suspended_tenant_and_a_disabled_membership` | Automated |
| T15 | `ac_au_06_no_active_membership_and_the_enumeration_defence`, `the_create_user_endpoint_is_rate_limited` | Automated |
| T16 | `ac_au_05_switch_rotates_the_session_and_audits_both_tenants` | Automated |
| T17 | `ac_id_04_a_revoke_during_a_job_cancels_it_and_closes_the_stream`, `ac_id_04_a_revoke_on_another_instance_closes_the_stream_within_five_seconds`, `ac_id_04_a_revoked_agent_is_refused`, `fr_id_04_a_revoked_agent_gets_no_rotated_certificate`, `ac_id_03_the_confirmation_refuses_the_old_certificate_and_a_job_continues`, `t17_a_superseded_certificate_writes_an_audit_event_and_marks_the_host`, `fr_id_03_a_confirmation_after_the_snapshot_keeps_the_stream` | Automated |
| T18 | `ac_id_02_a_database_dump_holds_no_ca_private_key`, `ac_pr_02_a_database_dump_holds_no_job_signing_key` | Automated |
| T19 | `t19_a_swapped_root_and_issuing_blob_is_refused`, `t19_a_swapped_signing_key_blob_is_refused`, `t19_a_swapped_retired_and_active_blob_is_refused` | Automated |
| T20 | `ac_ag_11_refuses_a_different_binary`, `ac_ag_11_refuses_a_missing_binary` | Automated |
| T21 | `ac_in_02_register_enrolls_the_host_and_stores_no_token`, `ac_ag_04_rejects_the_password_flag`, `t9_t21_the_enrollment_and_the_rotation_leave_no_key_or_token_in_the_database_or_the_logs` | Automated |
| T22 | `ac_au_17_the_right_gives_no_tenant_access` | Automated |
| T23 | `add_membership_writes_both_audit_records`, `a_role_change_and_a_removal_are_in_the_platform_audit` | Automated |
| T24 | `ac_tn_08_the_platform_tenant_is_hidden`, `ac_au_12_13_the_membership_list_holds_real_organizations_only` | Automated |
| T25 | `ac_au_02_a_lower_role_is_refused_in_the_platform_tenant`, `ac_au_16_the_self_right_and_the_last_admin_stay` | Automated |
| T26 | `ac_au_19_a_grant_requires_a_new_password_and_ends_the_sessions` | Automated |
| T27 | `fr_pr_09_an_agent_without_restore_v1_gets_no_ls_and_no_restore` | Automated |
| T28 | `ac_mn_05_webhook_url_policy_at_the_api`, `ac_mn_05_dns_name_to_a_private_address_is_refused`, `ac_mn_05_a_redirect_is_not_followed` | Automated |
| T29 | `ac_mn_08_secrets_stay_out_of_the_api_and_the_audit`, `ac_mn_08_the_secret_api_refuses_the_webhook_purposes`, `ac_mn_05_the_test_send_is_rate_limited` | Automated |
| T30 | `ac_mn_07_db_an_open_alert_is_unique_for_its_key`, `ac_mn_07_db_the_outbox_is_bounded` | Automated |
| T31 | `ac_rs_05_a_target_outside_the_roots_or_on_the_agent_files_is_rejected`, `ac_rs_05_a_link_to_the_configuration_directory_is_rejected`, `ac_rs_05_a_target_outside_the_agent_roots_is_refused_before_a_job_exists` | Automated |
| T32 | `adr_0035_a_browse_job_writes_an_audit_record_with_the_target_agent`, `adr_0035_browse_requests_have_an_actor_limit_and_an_agent_cap` | Automated |

The rotation of the agent certificate has tests too: `ac_id_03_an_agent_keeps_working_past_the_lifetime_through_two_rotations` (the clock moves across two rotations), `ac_id_03_the_agent_rotates_twice_over_its_open_stream` (the real agent on a real stream) and `fr_id_03_a_bad_rotation_request_is_refused_and_changes_nothing`.

The bench scenarios S1 to S3 of AC-ID-06 and NFR-PERF-04 run outside the unit test run and are not part of this table.

## 4. Recommendations for operators

1. Turn on bucket versioning and lifecycle rules on S3 repositories.
2. Give each host group its own backend credentials.
3. Run the maintenance agent on a dedicated host.
4. Copy important repositories (`restic copy`) to a second repository that agents cannot access.
5. Keep the KEK outside the database backup. It wraps the CA keys, the job signing keys and every secret. Without the KEK, all of them are lost.
