# ResticHub acceptance use cases

Status: for manual acceptance. Date: 2026-10-13.

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
- **Expected result:** the screen reports the success and shows the sign-in screen. The Platform Admin account exists in the platform tenant.
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
- **Expected result:** the dashboard appears. The top bar shows the organization switcher and the role of the person. The audit log holds a `sign_in` record.
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
- **Expected result:** the toast "Switched to name" appears. Every screen reloads its data for the new organization. The audit log holds a `session_switch` record.
- **Alternative flows:**
  - A failed switch shows a toast with the error.
  - A lost session redirects to the sign-in screen.
- **Requirements:** FR-AU-04, FR-TN-02.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

## 6. Platform Admin manages organizations

### UC-ORG-01: Create an organization

- **Actor:** the Platform Admin.
- **Preconditions:** the person signs in as the Platform Admin.
- **Main flow:**
  1. Open the "Organizations" screen in the "Platform" section.
  2. Click "Create an organization".
  3. Enter the name. The name holds 1 to 63 characters from `a-z`, `0-9` and `-`, starts with a letter and is not `platform`.
  4. Enter the username, the display name and a password of the first Admin. The password has 12 characters or more.
  5. Click "Create".
- **Expected result:** the list shows the new organization with the status "Active" and the user count 1. The first Admin signs in to the new organization.
- **Alternative flows:**
  - An existing name gives `conflict`.
  - An invalid name or password gives `validation_failed` on the field.
- **Requirements:** FR-TN-01, FR-TN-02.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

### UC-ORG-02: Suspend, resume and delete an organization

- **Actor:** the Platform Admin.
- **Preconditions:** the person signs in as the Platform Admin. The organization exists.
- **Main flow:**
  1. Open the "Organizations" screen.
  2. Click "Suspend" on an active organization.
  3. Click "Resume" to make the organization active again.
  4. Click "Delete" on a suspended organization and confirm.
- **Expected result:** a suspended organization refuses the sign-in and the API tokens of its users. Its agents stay connected. The status of a deleted organization is "Deleted". The platform audit log holds `tenant_suspend`, `tenant_resume` and `tenant_delete`.
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
- **Expected result:** a toast reports the switch. Every screen reloads its data for the new organization. The audit log holds a `session_switch` record.
- **Alternative flows:**
  - A Platform Admin membership appears under a Platform heading in the same menu.
  - A filter field appears in the menu when the account holds more than seven memberships.
- **Requirements:** FR-TN-02.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

## 7. Tenant admin manages users and roles

### UC-USR-01: Create a user with a role

- **Actor:** the Admin of the organization.
- **Preconditions:** the person signs in as the Admin.
- **Main flow:**
  1. Open the "Users" screen.
  2. Click "Create a user".
  3. Enter the username, the display name and a password of 12 characters or more.
  4. Select the role: Admin, Operator or Viewer.
  5. Click "Create".
- **Expected result:** the list shows the user with the role and the status "Enabled". The audit log holds `user_create`.
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
  1. Open the "Users" screen.
  2. Click "Disable" on an enabled user.
  3. Click "Enable" to restore the user.
- **Expected result:** the status changes between "Disabled" and "Enabled". A disabled user cannot sign in. The audit log holds `user_update`.
- **Requirements:** FR-TN-02.
- **Acceptance criteria:** AC-AU-02.
- **Status:** Ready on the stage.

## 8. Agent enrollment and host registration

### UC-AGT-01: Enroll a host

- **Actor:** the Admin creates the token. A person on the host runs the installation.
- **Preconditions:** the person signs in as the Admin. A Windows or Linux host is ready.
- **Main flow:**
  1. Open the "Agents" screen and create an enrollment token with labels.
  2. Copy the command. The screen shows the full registration command, with the token, one time.
  3. On the host, unpack the agent package.
  4. Run the command, for example `restichub-agent register --url <control-plane-url> --token <token>`. A script can use `--token-file` or the standard input.
  5. The agent generates its key pair and sends the certificate signing request and the token to the control plane.
  6. The control plane signs the certificate and returns it. The agent stores its identity in the protected configuration directory.
  7. Run `restichub-agent install-service` to register the operating system service.
- **Expected result:** the host appears in the "Agents" list with the status "Active". The certificate holds the agent ID and the tenant ID. The audit log holds `agent_enrolled`.
- **Alternative flows:**
  - A used or an expired token is refused with a clear error.
  - The 11th enrollment request from one address in one minute gives HTTP 429.
- **Requirements:** FR-ID-01, FR-ID-02, FR-ID-05, FR-IN-01, FR-IN-07, FR-IN-08.
- **Acceptance criteria:** AC-ID-01, AC-ID-05, AC-IN-02, AC-IN-03.
- **Status:** Blocked by #1. The child issues are #46 (enrollment tokens), #47 (agent registry) and #48 (enrollment exchange).

### UC-AGT-02: Revoke an agent

- **Actor:** the Admin of the organization.
- **Preconditions:** the person signs in as the Admin. The host is enrolled.
- **Main flow:**
  1. Open the "Agents" screen.
  2. Click "Revoke" on the host and confirm.
- **Expected result:** the control plane closes the stream in 5 s or less. The next connection is refused. The audit log holds `agent_revoke`.
- **Requirements:** FR-ID-04.
- **Acceptance criteria:** AC-ID-04.
- **Status:** Blocked by #1. The child issues are #47 (agent registry) and #52 (stream close).

## 9. Repository and secret configuration

### UC-RP-01: Store a repository secret

- **Actor:** an Admin writes a secret. A Viewer reads the metadata.
- **Preconditions:** the person signs in to a real organization. The platform tenant cannot hold a secret.
- **Main flow:**
  1. Create a secret with the name, the purpose and the value through `POST /api/v1/secrets`.
  2. List the secrets with `GET /api/v1/secrets`.
  3. Replace the value with `PUT /api/v1/secrets/{id}/value`.
- **Expected result:** the list shows the metadata and the marker `set`. No endpoint returns the value. The audit log holds `secret_create` and `secret_replace`.
- **Requirements:** FR-RP-03, FR-RP-05.
- **Acceptance criteria:** AC-RP-02.
- **Note:** the web UI screen is not built yet (#26). The person uses the REST API.
- **Status:** Ready on the stage.

### UC-RP-02: Add and initialize a repository

- **Actor:** an Admin.
- **Preconditions:** the person signs in. A backend is ready: S3, rest-server or a local path.
- **Main flow:**
  1. Open the "Repositories" screen and click "Create".
  2. Enter the backend address and the credentials.
  3. Select an agent and click "Test access".
  4. Click "Initialize".
- **Expected result:** the repository is ready for a profile. The test with a correct password succeeds.
- **Alternative flows:**
  - A wrong password gives the status `config_error` (code 12) with a clear message.
- **Requirements:** FR-RP-01, FR-RP-02.
- **Acceptance criteria:** AC-RP-01.
- **Status:** Blocked by #5.

## 10. Backup plan and profile

### UC-PF-01: Create a backup profile

- **Actor:** an Admin or an Operator.
- **Preconditions:** the person signs in. A repository exists.
- **Main flow:**
  1. Open the "Profiles" screen and click "Create".
  2. Select the repository. Enter the paths for each operating system, the excludes and the tags.
  3. Open the preview.
  4. Click "Save".
- **Expected result:** the preview shows the steps and the equivalent restic command line. The secrets are masked.
- **Alternative flows:**
  - An incorrect cron, a missing hook or an option that the installed restic lacks stops the save. The error points to the field.
- **Requirements:** FR-PF-01, FR-PF-04, FR-PF-05.
- **Acceptance criteria:** AC-PF-01, AC-PF-02.
- **Status:** Blocked by #6.

### UC-PL-01: Assign a plan with a schedule

- **Actor:** an Admin or an Operator.
- **Preconditions:** the person signs in. A profile exists. An agent is enrolled.
- **Main flow:**
  1. Open the "Plans" screen and click "Create".
  2. Select the profile and the agents.
  3. Enter the cron schedule and the time zone.
  4. Click "Save".
- **Expected result:** the plan appears in the list. The control plane creates a job at each fire time.
- **Requirements:** FR-SC-01, FR-SC-11.
- **Acceptance criteria:** AC-SC-01.
- **Status:** Blocked by #7.

## 11. Scheduled and manual backup job

### UC-JOB-01: A scheduled backup job runs

- **Actor:** the operator watches. The control plane does the work.
- **Preconditions:** a plan with a schedule. An online agent.
- **Main flow:**
  1. Wait for the fire time of the plan.
  2. Open the "Jobs" screen.
  3. Open the job and read the log.
- **Expected result:** the job history shows the status `success` and the restic summary. The dashboard shows the time of the last successful backup.
- **Requirements:** FR-SC-01, FR-SC-02, FR-AG-05.
- **Acceptance criteria:** AC-SC-01, AC-SC-02.
- **Status:** Blocked by #7.

### UC-JOB-02: Run a manual backup and cancel it

- **Actor:** an Operator.
- **Preconditions:** the person signs in. An online agent and a plan are ready.
- **Main flow:**
  1. Open the plan or the agent and click "Run".
  2. Watch the live log.
  3. Click "Cancel".
- **Expected result:** restic starts in 3 s or less. The cancel stops restic before the grace period ends. The status is `cancelled`. No lock stays on the repository.
- **Requirements:** FR-SC-09, FR-AG-10.
- **Acceptance criteria:** AC-SC-09, AC-AG-06.
- **Status:** Blocked by #7.

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
- **Status:** Blocked by #5.

## 15. Hooks from the allow-list

### UC-HK-01: Run a hook from the allow-list

- **Actor:** the host administrator enables the hook. An Admin or an Operator uses it in a profile.
- **Preconditions:** an enrolled host has the allow-list file.
- **Main flow:**
  1. On the host, add a hook to the allow-list file. Only root or Administrators write this file.
  2. In the profile, select the hook and enter its parameters.
  3. Save the profile and run the plan.
- **Expected result:** the agent validates the parameters against the schema and starts the hook without a shell. The hook runs in its phase: `before`, `after`, `on_success`, `on_error` or `finally`.
- **Alternative flows:**
  - An unknown hook ID is refused.
  - A `before` hook that fails stops the backup. The `on_error` and `finally` hooks run.
- **Requirements:** FR-HK-01, FR-HK-03, FR-HK-05.
- **Acceptance criteria:** AC-HK-01, AC-HK-03, AC-HK-06.
- **Status:** Blocked by #3.

## 16. Audit review

### UC-AUD-01: Review the tenant audit log

- **Actor:** an Admin, an Operator or a Viewer.
- **Preconditions:** the person signs in to an organization.
- **Main flow:**
  1. Open the "Audit log" screen.
  2. Set the filters: time, action, actor and object.
  3. Open one record.
- **Expected result:** each record shows the actor, the time, the IP address and the change. A secret value is redacted. No endpoint changes or deletes a record.
- **Requirements:** FR-AU-01.
- **Acceptance criteria:** AC-AU-01.
- **Status:** Ready on the stage.

### UC-AUD-02: Review the platform audit log

- **Actor:** the Platform Admin.
- **Preconditions:** the person signs in as the Platform Admin.
- **Main flow:**
  1. Open the "Platform audit" screen.
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
  1. Open the "API tokens" screen and click "Create a token".
  2. Enter the name, the role and the expiry date.
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
  1. Open the "API tokens" screen.
  2. Click "Revoke" on the token and confirm.
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
- **Expected result:** the missed-run policy applies. The policy "run all" creates 3 jobs, "run latest" creates 1 job and "skip" creates 0 jobs. An expired job gets the status `expired`.
- **Requirements:** FR-SC-07.
- **Acceptance criteria:** AC-SC-07.
- **Status:** Blocked by #7.

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
- **Status:** Blocked by #5.

### UC-FAIL-03: Wrong secret

- **Actor:** an Admin.
- **Preconditions:** a repository. A wrong repository password.
- **Main flow:**
  1. Save a wrong repository password in the secret.
  2. Test the access from an agent.
- **Expected result:** the test gives the status `config_error` (code 12) with a clear message. No secret value appears in a log.
- **Requirements:** FR-RP-02, FR-RP-05.
- **Acceptance criteria:** AC-RP-01.
- **Status:** Blocked by #5.

## 19. Traceability

| Use case | FR | AC | Status |
|---|---|---|---|
| UC-SETUP-01 | FR-AU-03, FR-TN-02 | AC-TN-06 | Ready on the stage |
| UC-SETUP-02 | FR-AU-03, FR-AU-04 | AC-AU-01 | Ready on the stage |
| UC-SETUP-03 | FR-AU-04, FR-TN-02 | AC-AU-01 | Ready on the stage |
| UC-ORG-01 | FR-TN-01, FR-TN-02 | AC-AU-01 | Ready on the stage |
| UC-ORG-02 | FR-TN-02 | AC-TN-03 | Ready on the stage |
| UC-ORG-03 | FR-TN-02 | AC-AU-01 | Ready on the stage |
| UC-USR-01 | FR-TN-02, FR-AU-03 | AC-AU-02 | Ready on the stage |
| UC-USR-02 | FR-TN-02 | AC-AU-02 | Ready on the stage |
| UC-AGT-01 | FR-ID-01, FR-ID-02, FR-ID-05, FR-IN-01, FR-IN-07, FR-IN-08 | AC-ID-01, AC-ID-05, AC-IN-02, AC-IN-03 | Blocked by #1 |
| UC-AGT-02 | FR-ID-04 | AC-ID-04 | Blocked by #1 |
| UC-RP-01 | FR-RP-03, FR-RP-05 | AC-RP-02 | Ready on the stage |
| UC-RP-02 | FR-RP-01, FR-RP-02 | AC-RP-01 | Blocked by #5 |
| UC-PF-01 | FR-PF-01, FR-PF-04, FR-PF-05 | AC-PF-01, AC-PF-02 | Blocked by #6 |
| UC-PL-01 | FR-SC-01, FR-SC-11 | AC-SC-01 | Blocked by #7 |
| UC-JOB-01 | FR-SC-01, FR-SC-02, FR-AG-05 | AC-SC-01, AC-SC-02 | Blocked by #7 |
| UC-JOB-02 | FR-SC-09, FR-AG-10 | AC-SC-09, AC-AG-06 | Blocked by #7 |
| UC-JOB-03 | FR-MN-01, FR-MN-02 | AC-MN-04, AC-UI-02 | Blocked by #9 |
| UC-RS-01 | FR-RS-01, FR-RS-03, FR-RS-04 | AC-RS-01, AC-RS-03, AC-RS-04 | Blocked by #8 |
| UC-MNT-01 | FR-RP-08, FR-SC-04, FR-SC-06 | AC-RP-04, AC-SC-03, AC-SC-06 | Blocked by #5 |
| UC-HK-01 | FR-HK-01, FR-HK-03, FR-HK-05 | AC-HK-01, AC-HK-03, AC-HK-06 | Blocked by #3 |
| UC-AUD-01 | FR-AU-01 | AC-AU-01 | Ready on the stage |
| UC-AUD-02 | FR-AU-01, FR-TN-02 | AC-AU-01 | Ready on the stage |
| UC-TOK-01 | FR-API-01, FR-TN-05 | AC-TN-04 | Ready on the stage |
| UC-TOK-02 | FR-API-01 | AC-TN-04 | Ready on the stage |
| UC-FAIL-01 | FR-SC-07 | AC-SC-07 | Blocked by #7 |
| UC-FAIL-02 | FR-MN-03 | AC-MN-02 | Blocked by #5 |
| UC-FAIL-03 | FR-RP-02, FR-RP-05 | AC-RP-01 | Blocked by #5 |
