# ResticHub acceptance use cases

Status: for manual acceptance. Date: 2026-10-17.

## 1. Purpose

This document holds the acceptance use cases of ResticHub. A use case is one scenario at the level of a person. The person does the steps in the web user interface or on a host. The result of each use case is observable.

The use cases connect the requirements and the manual acceptance. The [requirements](requirements.md) hold the functional requirement (FR) IDs. The [acceptance criteria](acceptance-criteria.md) hold the acceptance criterion (AC) IDs. Each use case names the FR IDs and the AC IDs that it covers.

A use case covers one scenario. It does not repeat each criterion of a requirement. A criterion with method A (automated test) stays in CI. This document holds the scenario-level checks that a person does by hand.

## 2. Identifiers

The identifier of a use case has the form `UC-<AREA>-NN`. `AREA` is the area of the use case, for example `ORG` for the organization administration. `NN` is a two-digit number that starts at 01.

## 3. Status

| Status | Meaning |
|---|---|
| Ready on the stage | The feature is in the code on the `main` branch now. The person can complete the use case on the stage. |
| Blocked by #N | The feature is not in the code. The GitHub issue #N delivers it. |

The status refers to the code, not to the plan. The check reads the routes in `crates/restichub-cp/src/routes.rs` and the screens in `ui/src/routes`.

## 4. The stage

The stage address is given to the testers outside the repository. It runs in multi-tenant mode with the image of the `main` branch. A Windows host with Docker is available for the benches.

The sign-in screen asks for the username and the password only. The account is installation-wide, so the person selects the organization in the top bar after the sign-in (ADR-0025).

## 5. First run and sign-in

### UC-SETUP-01: Complete the first-run setup

- **Actor:** the operator of the installation. The person becomes the Platform Admin.
- **Preconditions:** the control plane runs. The setup is not done. The person reads the control plane log.
- **Main flow:**
  1. Open the stage address in a browser.
  2. The application shows the setup screen, because the setup is not done.
  3. Read the one-time setup token in the control plane log with `docker compose logs cp`.
  4. Enter the setup token, the username, the display name and a password of 12 characters or more.
  5. Click "Create the Platform Admin".
- **Expected result:** the screen reports the success and shows the sign-in screen. The Platform Admin account holds the Platform Admin right and no membership.
- **Alternative flows:**
  - A wrong setup token gives the error `invalid_setup_token`.
  - A repeated setup gives the error `setup_already_done`.
- **Requirements:** FR-AU-03, FR-TN-02.
- **Acceptance criteria:** AC-TN-06.
- **Status:** Ready on the stage.

### UC-SETUP-02: Sign in

- **Actor:** the Platform Admin, an Admin, an Operator or a Viewer.
- **Preconditions:** the account exists. The organization is active.
- **Main flow:**
  1. Open the stage address.
  2. Enter the username and the password.
  3. Click "Sign in".
- **Expected result:** the "Overview" appears. The top bar shows the organization switcher and the account menu. The audit log holds a `sign_in` record.
- **Alternative flows:**
  - A wrong username or password gives `invalid_credentials`.
  - Many attempts from one address give `rate_limited`.
- **Requirements:** FR-AU-03, FR-AU-04.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

### UC-SETUP-03: Switch the organization

- **Actor:** a person with more than one membership.
- **Preconditions:** the account exists. The person is signed in.
- **Main flow:**
  1. Open the organization menu in the top bar.
  2. The menu lists the memberships of the person with the role and a check mark on the current organization.
  3. Click another organization. The menu shows a filter when the person holds more than seven memberships.
- **Expected result:** the toast "You switched to <name>." appears. Every screen reloads its data for the new organization. The audit log holds a `session_switch` record.
- **Alternative flows:**
  - A failed switch shows a toast with the error.
  - A lost session redirects to the sign-in screen.
- **Requirements:** FR-AU-04, FR-TN-02.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

### UC-SETUP-04: Sign in with no organization

- **Actor:** a Platform Admin with no membership, or another account with no membership.
- **Preconditions:** the account exists. The account holds no membership in an active organization.
- **Main flow:**
  1. Open the stage address.
  2. Enter the username and the password.
  3. Click "Sign in".
- **Expected result:** the sign-in succeeds. A Platform Admin lands in the platform mode (Organizations, Users, Audit). Another account sees the empty state "no organizations, ask an administrator". The switcher shows no organization and no Platform entry for that account, and the Platform administration entry for the Platform Admin.
- **Alternative flows:**
  - The platform screens appear for a Platform Admin in any current organization.
- **Requirements:** FR-AU-04, FR-TN-02.
- **Acceptance criteria:** AC-AU-06, AC-AU-11, AC-AU-12.
- **Status:** Ready on the stage.

## 6. Platform Admin manages organizations

### UC-ORG-01: Create an organization

- **Actor:** the Platform Admin.
- **Preconditions:** the person signs in as the Platform Admin.
- **Main flow:**
  1. Open the organization menu in the top bar and click "Platform administration".
  2. Open "Organizations" and click "Create organization".
  3. Enter the name. The name holds 1 to 63 characters from `a-z`, `0-9` and `-`, starts with a letter and is not `platform`.
  4. Click "Create organization".
- **Expected result:** the tab "Active" shows the new organization with "0 users" and the mark "No administrator". The platform audit log holds `tenant_create`.
- **Alternative flows:**
  - An existing name gives `conflict`.
  - An invalid name gives `validation_failed` on the field.
- **Requirements:** FR-TN-01, FR-TN-02.
- **Acceptance criteria:** AC-TN-07.
- **Status:** Ready on the stage.

### UC-ORG-02: Suspend, resume and delete an organization

- **Actor:** the Platform Admin.
- **Preconditions:** the person signs in as the Platform Admin. The organization exists.
- **Main flow:**
  1. Open "Organizations".
  2. In the menu "…" of an active organization, click "Suspend".
  3. In the same menu, click "Resume" to make the organization active again.
  4. Suspend the organization again. In its menu "…", click "Delete".
  5. Type the name of the organization and click "Delete organization".
- **Expected result:** a suspended organization refuses the sign-in and the API tokens of its users. Its agents stay connected. The tab "Deleted" shows a deleted organization. The platform audit log holds `tenant_suspend`, `tenant_resume` and `tenant_delete`.
- **Alternative flows:**
  - The platform tenant cannot be suspended or deleted (conflict).
  - The delete of an active organization is refused. Suspend it first.
- **Requirements:** FR-TN-02.
- **Acceptance criteria:** AC-TN-03.
- **Status:** Ready on the stage.

### UC-ORG-03: Switch the organization

- **Actor:** a person with a membership in more than one organization.
- **Preconditions:** the person is signed in. The account holds memberships in two or more organizations.
- **Main flow:**
  1. Click the organization name in the top bar.
  2. The menu lists the memberships with the role and a mark on the current organization.
  3. Click another organization.
- **Expected result:** a toast reports the switch. Every screen reloads its data for the new organization. The audit log holds a `session_switch` record. The menu lists real organizations only; the internal platform tenant appears in no entry.
- **Alternative flows:**
  - A Platform Admin with no membership sees the empty state in the menu.
  - A filter field appears in the menu when the account holds more than seven memberships.
- **Requirements:** FR-TN-02.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

## 7. Tenant admin manages users and roles

### UC-USR-01: Create a user with a role

- **Actor:** the Admin of the organization.
- **Preconditions:** the person signs in as the Admin.
- **Main flow:**
  1. Open "Settings" → "Access" → "Users".
  2. Click "Create user".
  3. Enter the username, the display name and a password of 12 characters or more.
  4. Select the role: Admin, Operator or Viewer.
  5. Click "Create user".
- **Expected result:** the list shows the user with the role. The audit log holds `user_create`.
- **Alternative flows:**
  - An existing username gives `conflict`.
  - An invalid field gives `validation_failed`.
- **Requirements:** FR-TN-02, FR-AU-03.
- **Acceptance criteria:** AC-AU-02.
- **Status:** Ready on the stage.

### UC-USR-02: Disable and enable a user

- **Actor:** the Admin of the organization.
- **Preconditions:** the person signs in as the Admin. The user exists.
- **Main flow:**
  1. Open "Settings" → "Access" → "Users".
  2. In the menu "…" of an enabled user, click "Disable user" and confirm.
  3. In the same menu, click "Enable user" to restore the user.
- **Expected result:** the user shows the mark "Disabled", and the mark goes away after "Enable user". A disabled user cannot sign in. The audit log holds `user_update`.
- **Requirements:** FR-TN-02.
- **Acceptance criteria:** AC-AU-02.
- **Status:** Ready on the stage.

### UC-USR-03: Create an installation account and add it to an organization

- **Actor:** the Platform Admin.
- **Preconditions:** the person signs in as the Platform Admin. An organization exists.
- **Main flow:**
  1. Open "Platform administration" → "Users".
  2. Click "Create user".
  3. Enter the username, the display name and a password of 12 characters or more. Enter an optional email.
  4. Add a membership: select an organization and a role.
  5. Click "Create user".
- **Expected result:** the list shows the account. Its page shows the memberships and the roles. The person signs in and reaches the organization. The platform audit log holds `user_create`.
- **Alternative flows:**
  - An existing username gives `username_taken`. An existing email gives `email_taken`.
  - An organization without an active Admin shows the mark "No administrator".
- **Requirements:** FR-TN-02, FR-AU-04.
- **Acceptance criteria:** AC-AU-08, AC-AU-09.
- **Status:** Ready on the stage.

## 8. Agent enrollment and host registration

### UC-AGT-01: Enroll a host

- **Actor:** the Admin creates the token. A person on the host runs the installation.
- **Preconditions:** the person signs in as the Admin. A Windows or Linux host is ready.
- **Main flow:**
  1. Open "Hosts" and click "Add host".
  2. Enter the labels and click "Create token". The dialog "Add a host" shows the token and the install command one time.
  3. Copy the command. The token stays under "Pending registrations" until a host uses it.
  4. On the host, unpack the agent package.
  5. Run the command as root or Administrator, for example `restichub-agent register --url <control-plane-url> --token <token>`. A script can use `--token-file` or `--token-stdin`.
  6. The agent generates its key pair and sends the certificate signing request and the token to the control plane.
  7. The control plane signs the certificate and returns it. The agent stores its identity in the protected configuration directory.
  8. The command installs and starts the service and waits for the connection.
- **Expected result:** the host appears in the "Hosts" list with the status "Online". The pending registration disappears. The certificate holds the agent ID and the tenant ID. The audit log holds `enrollment_token_create` and `agent_enroll`.
- **Alternative flows:**
  - A used, an expired or a revoked token is refused with a clear error. The Admin can click "Cancel" on a pending registration to revoke its token.
  - The 11th enrollment request from one address in one minute gives HTTP 429.
  - The Admin can click "Rename" in the menu "…" of a host to change its name.
- **Requirements:** FR-ID-01, FR-ID-02, FR-ID-05, FR-IN-01, FR-IN-07, FR-IN-08.
- **Acceptance criteria:** AC-ID-01, AC-ID-05, AC-IN-02, AC-IN-03.
- **Status:** Ready on the stage. The epic #1 stays open for the other protocol work.

### UC-AGT-02: Revoke an agent

- **Actor:** the Admin of the organization.
- **Preconditions:** the person signs in as the Admin. The host is enrolled.
- **Main flow:**
  1. Open "Hosts".
  2. In the menu "…" of the host, click "Revoke agent" and confirm.
- **Expected result:** the status of the host changes to "Revoked". The control plane closes the stream in 5 s or less and sends a cancellation for a running job. The next connection is refused. The audit log holds `agent_revoke`.
- **Requirements:** FR-ID-04.
- **Acceptance criteria:** AC-ID-04.
- **Status:** Ready on the stage. The issue #52 stays open for the revocation check at each connect.

### UC-AGT-03: Update the agent package in place

- **Actor:** the host administrator.
- **Preconditions:** the host is enrolled and the agent service runs. A newer agent package is available.
- **Main flow:**
  1. Unpack the new agent package in an empty directory.
  2. Run `restichub-agent install-service` from the new package as root or Administrator. Do not run `register` again.
- **Expected result:** the command stops the service, copies the new program, starts the service and waits for the connection. The agent connects with its old identity. The "Hosts" list shows the same host with the status "Online" and the new version in the second line of the row. The host keeps its agent ID, its certificate and its trust mode file. The next job of the host runs with the new files.
- **Alternative flows:**
  - A `restic` file that does not match the pinned hash stops the agent at start. The host stays offline and the service log gives the cause.
  - A job that the agent already ran does not run again after the start. The agent journal holds the job.
- **Requirements:** FR-IN-07, FR-IN-08, FR-HK-01.
- **Acceptance criteria:** AC-IN-01, AC-HK-02.
- **Note:** the update is a manual step in the MVP. The automatic self-update (FR-AG-13) is planned for 1.0. No criterion covers the in-place update of the package as a whole. The listed criteria cover the package content and the checksum of the trust mode file.
- **Status:** Ready on the stage.

### UC-AGT-04: Set the parameter values of a host

- **Actor:** an Admin or an Operator.
- **Preconditions:** the person signs in. The host is enrolled. A profile with a parameter has a plan on the host.
- **Main flow:**
  1. Open "Hosts" and open the host. The section "Parameters" lists the parameters of the profiles of the host.
  2. Enter the value of a text parameter, for example the mount point of the backup path.
  3. Select a secret of the purpose `hook_secret` as the value of a secret parameter.
  4. Click "Save".
- **Expected result:** the section shows the parameters with the marker "Set". The plans of the host use the values. The audit log holds the names of the parameters and the identifiers of the secrets only. The plan save of this host needs no value for a parameter that the host holds.
- **Alternative flows:**
  - A new value that leaves a plan without a required value is refused with `parameter_missing_in_plans` and the list of the plans.
  - A value of the wrong kind is refused with `parameter_kind_mismatch`.
  - A Viewer sees the names of the parameters without the values.
  - A secret that a value references cannot be deleted.
- **Requirements:** FR-PF-10, FR-PF-11, FR-PF-12.
- **Acceptance criteria:** AC-PF-07, AC-PF-10.
- **Status:** Blocked by #283.

## 9. Repository and secret configuration

### UC-RP-01: Store a repository secret

- **Actor:** an Admin writes a secret. A Viewer reads the metadata.
- **Preconditions:** the person signs in to a real organization. The platform tenant cannot hold a secret.
- **Main flow:**
  1. Create a secret with the name, the purpose and the value through `POST /api/v1/secrets`.
  2. List the secrets with `GET /api/v1/secrets`.
  3. Replace the value with `PUT /api/v1/secrets/{id}/value`.
- **Expected result:** the list shows the metadata and the marker `set`. No endpoint returns the value. The audit log holds `secret_create` and `secret_replace`.
- **Alternative flows:**
  - The page of a repository also writes the secrets of that repository: the repository password, the S3 keys and the REST server password. It shows a stored value never again.
- **Requirements:** FR-RP-03, FR-RP-05.
- **Acceptance criteria:** AC-RP-02.
- **Note:** "Settings" → "Access" → "Secrets" lists the secrets and replaces a value. The REST API does the same steps.
- **Status:** Ready on the stage.

### UC-RP-02: Add and initialize a repository

- **Actor:** an Admin.
- **Preconditions:** the person signs in. A backend is ready: S3, rest-server or a local path. A host is enrolled.
- **Main flow:**
  1. Open "Repositories" and click "Add repository". The page "New repository" opens.
  2. In the section "Location", select the backend kind: "S3-compatible", "REST server" or "Local path". Enter the backend address.
  3. In the section "Access", enter the repository password and the credentials.
  4. Select the "Maintenance host". For a local path, the path is valid on this host only.
  5. Click "Save". The list shows the repository with the mark "Not initialized".
  6. In the menu "…" of the repository, click "Test access". Select the host and click "Start".
  7. In the same menu, click "Initialize". Read the warning about the password and click "Start". The initialization runs on the maintenance host.
- **Expected result:** the dialog shows the job status and the result "The repository is ready.". The mark "Not initialized" goes away. The test with a correct password succeeds. The audit log holds `repository_create`.
- **Alternative flows:**
  - A wrong password or wrong credentials give the status `config_error` (code 12) and the message "The repository password or the credentials are not correct.".
  - A repository that does not exist yet gives the result "The repository does not exist yet. Initialize it.".
  - A local path on a host that does not hold the repository gives the result `access_error`.
  - The location of an initialized repository cannot change.
- **Requirements:** FR-RP-01, FR-RP-02.
- **Acceptance criteria:** AC-RP-01.
- **Note:** the issue #73 (maintenance agent assignment and barriers) is open. The maintenance host is a setting of the repository now, and the barriers are not in the code.
- **Status:** Ready on the stage.

### UC-RP-03: Remove all locks of a repository

- **Actor:** an Admin.
- **Preconditions:** the person signs in as the Admin. An initialized repository has a lock that no operation holds, for example after a killed restic. The Admin knows that no other operation runs on the repository.
- **Main flow:**
  1. Open "Repositories". In the menu "…" of the repository, click "Unlock all".
  2. Read the warning, type the repository name and confirm. The REST API does the same step: `POST /api/v1/repositories/{id}/unlock-all` with the body `{"confirm": "<repository name>"}`.
  3. Open "Jobs" and find the job of the kind "Unlock all".
- **Expected result:** the call queues one `unlock_all` job. The agent runs `restic unlock --remove-all`. The job gets the status "Succeeded". The audit log holds `repository_unlock_all` with the actor.
- **Alternative flows:**
  - A name that differs from the repository name is refused and no job starts.
  - An Operator or a Viewer gets HTTP 403. Only an Admin runs the action, and only an Admin cancels the job.
  - A repository of another organization gives HTTP 404.
- **Requirements:** FR-SC-06, FR-PR-02.
- **Acceptance criteria:** AC-SC-06.
- **Status:** Ready on the stage.

## 10. Backup plan and profile

### UC-PF-01: Create a backup profile

- **Actor:** an Admin or an Operator.
- **Preconditions:** the person signs in.
- **Main flow:**
  1. Open "Profiles" and click "Add profile". The profile page opens.
  2. In the section "Main", enter the name and the description.
  3. In the section "What to back up", enter the paths for each operating system. Enter the excludes, the tags and the backup options in their sections.
  4. In the section "Parameters", declare the parameters. Give the name, the kind "Text" or "Secret", the required flag and the default value of a text parameter.
  5. Set the retention and the scripts when they are needed. The panel "The result" on the right side shows a summary.
  6. Click "Preview", select a plan and read the steps.
  7. Click "Save".
- **Expected result:** the preview shows the steps and the equivalent restic command line of the plan. The secrets and the parameter values are masked. The profile appears in the list. The audit log holds `profile_create`.
- **Alternative flows:**
  - An incorrect value or an option that the installed restic lacks stops the save. The error points to the field.
  - A profile that fits the host with a warning is saved. The screen shows "The profile is saved with warnings.".
- **Requirements:** FR-PF-01, FR-PF-03, FR-PF-04, FR-PF-05, FR-PF-09.
- **Acceptance criteria:** AC-PF-01, AC-PF-02, AC-PF-09.
- **Note:** the profile holds no repository. The plan selects the repository (UC-PL-01).
- **Status:** Blocked by #283.

### UC-PL-01: Assign a plan with a schedule

- **Actor:** an Admin or an Operator.
- **Preconditions:** the person signs in. A profile exists. An agent is enrolled. A repository exists.
- **Main flow:**
  1. Open "Plans" and click "Add plan". The plan page opens.
  2. In the section "What to back up", select the profile, the host and the repository.
  3. In the section "Parameters", read the required parameters. Enter the value of a parameter that the plan overrides. Leave the other values to the host or to the default.
  4. In the section "Schedule", enter the cron expression, or select a preset. Enter the time zone, for example `Europe/Berlin`.
  5. Select the policy for "Missed runs": "Run every missed run", "Run the latest missed run" or "Skip the missed runs".
  6. Read the "Next 5 runs" and click "Save".
- **Expected result:** the plan appears in the list. The panel on the plan page shows the "Next run" and the repository. The control plane creates a job at each fire time. The audit log holds `plan_create`.
- **Alternative flows:**
  - A required parameter without a value on the plan and on the host stops the save with `parameter_missing` and the names of the parameters.
  - A local repository whose host differs from the host of the plan stops the save with `not_repository_host`.
  - An incorrect cron expression or time zone stops the save. The error points to the field.
  - A plan that is disabled creates no job.
- **Requirements:** FR-SC-01, FR-SC-11, FR-PF-10, FR-PF-11.
- **Acceptance criteria:** AC-SC-01, AC-PF-06, AC-PF-08, AC-RP-07.
- **Status:** Blocked by #283.

## 11. Scheduled and manual backup job

### UC-JOB-01: A scheduled backup job runs

- **Actor:** the operator watches. The control plane does the work.
- **Preconditions:** a plan with a schedule. An online agent.
- **Main flow:**
  1. Wait for the fire time of the plan.
  2. Open the "Jobs" screen. The newest job is first.
  3. Read the status, the host, the plan, the start time and the duration of the job.
- **Expected result:** the job shows the status "Succeeded". The "Next run" on the plan page moves to the next fire time.
- **Alternative flows:**
  - A job that ends with exit code 3 shows "Succeeded" in yellow with the tooltip "Succeeded with warnings" (UC-JOB-05).
  - A job that fails shows "Failed" with the reason.
  - A job that the host does not confirm before its expiry shows "Expired".
- **Requirements:** FR-SC-01, FR-SC-02, FR-AG-05.
- **Acceptance criteria:** AC-SC-01, AC-SC-02.
- **Note:** the live log and the restic summary on the job screen are not on `main`. The epic #9 delivers them. The check of the log and the summary is blocked by #9. The dashboard time of the last successful backup is part of #9 too.
- **Status:** Ready on the stage for the scheduled start and the job status. Blocked by #9 for the log, the restic summary and the dashboard.

### UC-JOB-02: Run a manual backup and cancel it

- **Actor:** an Operator.
- **Preconditions:** the person signs in. An online agent and a plan are ready.
- **Main flow:**
  1. Open "Plans". In the menu "…" of the plan, click "Run now".
  2. Open "Jobs". The new job shows "Waiting", then "Sent to the host", then "Running".
  3. In the menu "…" of the job, or on the job page, click "Cancel job" and confirm.
- **Expected result:** restic starts in 3 s or less. The cancel stops restic before the grace period ends. The status is "Cancelled". No lock stays on the repository. The audit log holds `job_run` and `job_cancel`.
- **Alternative flows:**
  - A second "Run now" for a plan that already has a manual run in the queue gives `already_queued`.
  - An offline host gives `agent_offline`. The control plane holds no job queue for an offline host, so it refuses the job.
- **Requirements:** FR-SC-09, FR-AG-10.
- **Acceptance criteria:** AC-SC-09, AC-AG-06.
- **Note:** the live log is not on `main`. The epic #9 delivers it. The step "watch the live log" stays blocked by #9.
- **Status:** Ready on the stage for the run, the queue and the cancel. Blocked by #9 for the live log.

### UC-JOB-04: A locked repository retries and gives up

- **Actor:** the operator watches. The control plane does the work.
- **Preconditions:** a plan and an online agent. Another process holds a lock on the repository, for example a `restic` command on another computer. A test setup can use a fake restic that exits with code 11.
- **Main flow:**
  1. Run the plan with "Run now".
  2. Open the "Jobs" screen.
  3. Watch the jobs of the plan while the lock stays.
- **Expected result:** restic ends with exit code 11. The job shows the reason "The repository was locked. A new attempt is queued.". The control plane queues a new job. The new job has a new identifier and a longer delay than the one before. When the retry limit is reached, the last job gets the status "Gave up" (`dead_letter`) with the reason "The repository stayed locked after every retry.". The audit log holds `job_dead_letter`.
- **Alternative flows:**
  - If the lock goes away between two attempts, the new job runs and gets the status "Succeeded".
  - The Admin can remove a stale lock by hand (UC-RP-03).
- **Requirements:** FR-SC-05, FR-AG-09.
- **Acceptance criteria:** AC-SC-05, AC-AG-05.
- **Note:** the alert for the dead-letter job is part of the epic #9. The audit record is on `main` now.
- **Status:** Ready on the stage.

### UC-JOB-05: A partial backup ends with warnings

- **Actor:** the operator watches. The control plane does the work.
- **Preconditions:** a plan and an online agent. One file in the backup path cannot be read, for example a file that another process locks. A test setup can use a fake restic that exits with code 3.
- **Main flow:**
  1. Run the plan with "Run now", or wait for its fire time.
  2. Open the "Jobs" screen.
- **Expected result:** restic ends with exit code 3. The job shows "Succeeded" in yellow with the tooltip "Succeeded with warnings" (`succeeded_with_warnings`). The tab "With warnings" of the "Jobs" list shows it. The snapshot exists with the files that restic could read. The job does not retry.
- **Alternative flows:**
  - If no file can be read, restic gives another exit code and the job gets the status "Failed".
- **Requirements:** FR-AG-09.
- **Acceptance criteria:** AC-AG-05.
- **Status:** Ready on the stage.

## 12. Job monitoring and cancel

### UC-JOB-03: Watch the dashboard and the job history

- **Actor:** an Operator or a Viewer.
- **Preconditions:** the person signs in. Jobs exist.
- **Main flow:**
  1. Open the dashboard.
  2. Read the state of each agent and plan.
  3. Open the "Jobs" screen and open the statistics of a job.
- **Expected result:** the dashboard shows the last successful backup, the RPO state, the lock state and the online state. The statistics agree with the restic summary.
- **Requirements:** FR-MN-01, FR-MN-02.
- **Acceptance criteria:** AC-MN-04, AC-UI-02.
- **Status:** Blocked by #9.

## 13. Restore

### UC-RS-01: Restore a snapshot to another host

- **Actor:** an Operator.
- **Preconditions:** the person signs in. A snapshot exists. Two hosts are ready.
- **Main flow:**
  1. Open the "Restore" screen.
  2. Filter the snapshots by host, tag, path or date.
  3. Select a snapshot and open its file tree.
  4. Select the target agent and the target path.
  5. Click "Restore".
- **Expected result:** the target agent restores the files. The data does not go through the control plane. The SHA-256 of the files agrees with the source.
- **Alternative flows:**
  - The option `--dry-run` changes no file.
- **Requirements:** FR-RS-01, FR-RS-03, FR-RS-04.
- **Acceptance criteria:** AC-RS-01, AC-RS-03, AC-RS-04.
- **Status:** Blocked by #8.

## 14. Retention, prune and check

### UC-MNT-01: Run retention, prune and check

- **Actor:** an Admin or an Operator.
- **Preconditions:** the person signs in. A repository has a maintenance agent.
- **Main flow:**
  1. Set the retention in the profile.
  2. Let the plan run.
  3. Read the job log of the maintenance agent.
- **Expected result:** only the maintenance agent runs `forget`, `prune` and `check`. No backup runs at the same time on the repository. After a stale lock, the control plane runs `unlock` when the repository is idle.
- **Requirements:** FR-RP-08, FR-SC-04, FR-SC-06.
- **Acceptance criteria:** AC-RP-04, AC-SC-03, AC-SC-06.
- **Note:** the issue #73 (maintenance agent assignment and barriers) delivers the rule that only the maintenance agent runs these steps.
- **Status:** Blocked by #73.

## 15. Scripts on a trusted host

### UC-HK-01: Run a script before and after a backup

- **Actor:** the host administrator turns on the trust mode. An Admin writes the script in a profile.
- **Preconditions:** an enrolled host reports the capability `scripts-v1`.
- **Main flow:**
  1. On the host, run `restichub-agent trust-server-scripts --enable` as root or Administrator.
  2. In the profile, add a script. Select the interpreter, the phase and the timeout. Enter the body and the parameters.
  3. Save the profile and run the plan.
- **Expected result:** the job detail lists the script and the backup in order. The script runs in its phase: `before`, `after`, `on_success`, `on_error` or `finally`. Each step has its own log.
- **Alternative flows:**
  - A host without the trust mode refuses the job with a typed reason.
  - A `before` script with the policy `abort_backup` that fails stops the backup. The `on_error` and `finally` scripts run.
- **Requirements:** FR-HK-01, FR-HK-05, FR-HK-08, FR-HK-09, FR-HK-10.
- **Acceptance criteria:** AC-HK-02, AC-HK-07, AC-HK-13.
- **Status:** Blocked by #3.

## 16. Audit review

### UC-AUD-01: Review the tenant audit log

- **Actor:** an Admin, an Operator or a Viewer.
- **Preconditions:** the person signs in to an organization.
- **Main flow:**
  1. Open "Audit". The list shows the last 30 days, with a heading for each day.
  2. Search by the name of the object or the actor. Set the filters "Who", "Object", "Action" and "Period".
  3. Open one record.
- **Expected result:** each record shows the actor, the time, the IP address and the change. A secret value is redacted. No endpoint changes or deletes a record.
- **Requirements:** FR-AU-01.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

### UC-AUD-02: Review the platform audit log

- **Actor:** the Platform Admin.
- **Preconditions:** the person signs in as the Platform Admin.
- **Main flow:**
  1. Open "Platform administration" → "Audit".
  2. Read the records of the organization operations.
- **Expected result:** the records show `tenant_create`, `tenant_suspend`, `tenant_resume` and `tenant_delete` with the actor and the time.
- **Requirements:** FR-AU-01, FR-TN-02.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

## 17. API tokens

### UC-TOK-01: Create and use an API token

- **Actor:** an Admin.
- **Preconditions:** the person signs in to an organization.
- **Main flow:**
  1. Open "Settings" → "Access" → "API tokens" and click "Create token".
  2. Enter the name, select the role and the expiry: "Never", "In 30 days", "In 90 days" or "In 1 year".
  3. Copy the token value. The screen shows the value one time.
  4. Call the API with the token in the `Authorization: Bearer` header.
- **Expected result:** the token appears in the list with its prefix and expiry. The API answers the request. The audit log holds `api_token_create`.
- **Requirements:** FR-API-01, FR-TN-05.
- **Acceptance criteria:** AC-TN-04.
- **Status:** Ready on the stage.

### UC-TOK-02: Revoke an API token

- **Actor:** an Admin.
- **Preconditions:** the person signs in. A token exists.
- **Main flow:**
  1. Open "Settings" → "Access" → "API tokens".
  2. In the menu "…" of the token, click "Revoke" and confirm.
- **Expected result:** the token stops working. The next request gives HTTP 401. The audit log holds `api_token_revoke`.
- **Requirements:** FR-API-01.
- **Acceptance criteria:** AC-TN-04.
- **Status:** Ready on the stage.

## 18. Failure cases

### UC-FAIL-01: Agent offline during a scheduled run

- **Actor:** an Operator.
- **Preconditions:** a plan with a schedule. An agent.
- **Main flow:**
  1. Stop the agent service. Keep the agent offline for 3 plan intervals.
  2. Start the agent again.
  3. Open the "Jobs" screen.
- **Expected result:** the missed-run policy of the plan applies. The policy `run_all` ("Run every missed run") creates 3 jobs. The policy `latest_only` ("Run the latest missed run") creates 1 job. The policy `skip` ("Skip the missed runs") creates 0 jobs. A fire time within the grace window of the pass is on time. That run always runs, and it is in addition to the count. A job that the host does not take before its expiry gets the status "Expired".
- **Requirements:** FR-SC-07.
- **Acceptance criteria:** AC-SC-07.
- **Status:** Ready on the stage.

### UC-FAIL-02: Repository unavailable

- **Actor:** an Operator.
- **Preconditions:** a plan. A repository backend.
- **Main flow:**
  1. Make the repository backend unavailable.
  2. Run the plan.
  3. Read the job and the alerts.
- **Expected result:** the job fails with a clear error. An alert arrives with the tenant, the agent, the plan and the error text.
- **Requirements:** FR-MN-03.
- **Acceptance criteria:** AC-MN-02.
- **Note:** the job and its status are on `main`. The alerts are not. The epic #9 delivers them.
- **Status:** Blocked by #9.

### UC-FAIL-03: Wrong secret

- **Actor:** an Admin.
- **Preconditions:** a repository. A wrong repository password.
- **Main flow:**
  1. Create a repository with a wrong repository password. The password cannot change after the creation.
  2. Open the actions menu of the repository and click "Test access". Select a host and click "Start".
- **Expected result:** the test gives the status `config_error` (code 12) and the message "The repository password or the credentials are not correct.". No secret value appears in a log.
- **Alternative flows:**
  - A wrong S3 key or a wrong REST server password gives the same result. The Admin replaces these secrets in the form "Edit the repository" and runs the test again.
- **Requirements:** FR-RP-02, FR-RP-05.
- **Acceptance criteria:** AC-RP-01.
- **Status:** Ready on the stage.

### UC-TN-01: Tenant isolation

- **Actor:** an Admin of the organization A. The organization B exists.
- **Preconditions:** the person holds a membership in the organization A only. An object of the organization B exists, for example a repository, a plan or a job. The person knows its ID.
- **Main flow:**
  1. Sign in and select the organization A.
  2. Call `GET /api/v1/repositories/{id}` with the ID of the object of the organization B.
  3. Repeat the call for a plan, a profile, a job and a host. Try also a change, for example `PATCH` or `DELETE`, and the action `POST /api/v1/plans/{id}/run`.
- **Expected result:** each call gives HTTP 404, as if the object does not exist. No object of the organization B changes. The lists of the organization A show no object of the organization B.
- **Alternative flows:**
  - An API token of the organization A gives the same result.
  - After the person switches to the organization B (with a membership), the same IDs give the objects.
- **Requirements:** FR-TN-01.
- **Acceptance criteria:** AC-TN-01.
- **Status:** Ready on the stage.

## 19. Traceability

| Use case | FR | AC | Status |
|---|---|---|---|
| UC-SETUP-01 | FR-AU-03, FR-TN-02 | AC-TN-06 | Ready on the stage |
| UC-SETUP-02 | FR-AU-03, FR-AU-04 | AC-AU-01 | Ready on the stage |
| UC-SETUP-03 | FR-AU-04, FR-TN-02 | AC-AU-01 | Ready on the stage |
| UC-SETUP-04 | FR-AU-04, FR-TN-02 | AC-AU-06, AC-AU-11, AC-AU-12 | Ready on the stage |
| UC-ORG-01 | FR-TN-01, FR-TN-02 | AC-TN-07 | Ready on the stage |
| UC-ORG-02 | FR-TN-02 | AC-TN-03 | Ready on the stage |
| UC-ORG-03 | FR-TN-02 | AC-AU-01 | Ready on the stage |
| UC-USR-01 | FR-TN-02, FR-AU-03 | AC-AU-02 | Ready on the stage |
| UC-USR-02 | FR-TN-02 | AC-AU-02 | Ready on the stage |
| UC-USR-03 | FR-TN-02, FR-AU-04 | AC-AU-08, AC-AU-09 | Ready on the stage |
| UC-AGT-01 | FR-ID-01, FR-ID-02, FR-ID-05, FR-IN-01, FR-IN-07, FR-IN-08 | AC-ID-01, AC-ID-05, AC-IN-02, AC-IN-03 | Ready on the stage |
| UC-AGT-02 | FR-ID-04 | AC-ID-04 | Ready on the stage |
| UC-AGT-03 | FR-IN-07, FR-IN-08, FR-HK-01 | AC-IN-01, AC-HK-02 | Ready on the stage |
| UC-AGT-04 | FR-PF-10, FR-PF-11, FR-PF-12 | AC-PF-07, AC-PF-10 | Blocked by #283 |
| UC-RP-01 | FR-RP-03, FR-RP-05 | AC-RP-02 | Ready on the stage |
| UC-RP-02 | FR-RP-01, FR-RP-02 | AC-RP-01 | Ready on the stage |
| UC-RP-03 | FR-SC-06, FR-PR-02 | AC-SC-06 | Ready on the stage |
| UC-PF-01 | FR-PF-01, FR-PF-03, FR-PF-04, FR-PF-05, FR-PF-09 | AC-PF-01, AC-PF-02, AC-PF-09 | Blocked by #283 |
| UC-PL-01 | FR-SC-01, FR-SC-11, FR-PF-10, FR-PF-11 | AC-SC-01, AC-PF-06, AC-PF-08, AC-RP-07 | Blocked by #283 |
| UC-JOB-01 | FR-SC-01, FR-SC-02, FR-AG-05 | AC-SC-01, AC-SC-02 | Ready on the stage; blocked by #9 for the log, the summary and the dashboard |
| UC-JOB-02 | FR-SC-09, FR-AG-10 | AC-SC-09, AC-AG-06 | Ready on the stage; blocked by #9 for the live log |
| UC-JOB-04 | FR-SC-05, FR-AG-09 | AC-SC-05, AC-AG-05 | Ready on the stage |
| UC-JOB-05 | FR-AG-09 | AC-AG-05 | Ready on the stage |
| UC-JOB-03 | FR-MN-01, FR-MN-02 | AC-MN-04, AC-UI-02 | Blocked by #9 |
| UC-RS-01 | FR-RS-01, FR-RS-03, FR-RS-04 | AC-RS-01, AC-RS-03, AC-RS-04 | Blocked by #8 |
| UC-MNT-01 | FR-RP-08, FR-SC-04, FR-SC-06 | AC-RP-04, AC-SC-03, AC-SC-06 | Blocked by #73 |
| UC-HK-01 | FR-HK-01, FR-HK-05, FR-HK-08, FR-HK-09, FR-HK-10 | AC-HK-02, AC-HK-07, AC-HK-13 | Blocked by #3 |
| UC-AUD-01 | FR-AU-01 | AC-AU-01 | Ready on the stage |
| UC-AUD-02 | FR-AU-01, FR-TN-02 | AC-AU-01 | Ready on the stage |
| UC-TOK-01 | FR-API-01, FR-TN-05 | AC-TN-04 | Ready on the stage |
| UC-TOK-02 | FR-API-01 | AC-TN-04 | Ready on the stage |
| UC-FAIL-01 | FR-SC-07 | AC-SC-07 | Ready on the stage |
| UC-FAIL-02 | FR-MN-03 | AC-MN-02 | Blocked by #9 |
| UC-FAIL-03 | FR-RP-02, FR-RP-05 | AC-RP-01 | Ready on the stage |
| UC-TN-01 | FR-TN-01 | AC-TN-01 | Ready on the stage |
