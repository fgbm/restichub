---
title: Troubleshooting
description: Find the cause of a problem by its symptom, and the action that removes it.
---

Find the symptom in the tables below. The column "Action" tells what to do first. The link leads to the full description.

## Control plane

| Symptom | Cause | Action |
|---|---|---|
| `docker compose ps cp` does not show `(healthy)` | The control plane did not start. | Read `docker compose logs cp`. The last error line names the cause. |
| The control plane stops with an error about the key file | Another user than uid 10001 owns `secrets/restichub_kek`, or others can read it. | Run `sudo chown 10001:10001 secrets/restichub_kek` and `sudo chmod 400 secrets/restichub_kek`. See [The environment file](/restichub/getting-started/install-control-plane/#the-environment-file). |
| The browser shows a certificate error | Caddy did not get a certificate. A firewall blocks port 80 or 443, or the DNS record points to another address. | Open the ports and correct the DNS record. Read `docker compose logs caddy`. |
| The screen "Initial setup" asks for a token | The installation has no administrator. | Run `docker compose logs cp \| grep setup_token`. See [The first administrator](/restichub/getting-started/install-control-plane/#the-first-administrator). |
| The control plane stops with a message about the tenancy mode | The mode `single` found more than one active organization. | Set `RESTICHUB_TENANCY_MODE=multi`, or suspend the other organizations. See [Organizations](/restichub/guides/organizations/). |

## Hosts

| Symptom | Cause | Action |
|---|---|---|
| `register` stops with a message about the clock | The clock of the host differs by more than 5 minutes. | Correct the clock. See [The host clock](/restichub/getting-started/add-a-host/#the-host-clock). |
| `register` stops: the control plane refuses the token | A host already used the token, the token expired, or an Admin revoked it. | Create a new token with "Add host". |
| `register` gets HTTP 429 | Too many enrollment requests from one address. | Wait one minute. See [Agent enrollment rate limit](/restichub/operations/enrollment-rate-limit/). |
| The host stays in "Waiting for connection" | The service does not run, or it cannot reach port 8443. | Run `status` and read "last error". Check the firewall to port 8443. |
| The host is "Offline" | The service stopped, or the network to the control plane is down. | Read the log of the agent. See [Agent logs](/restichub/reference/agent-logs/). |
| The host shows "Agent too old" | The control plane refuses the protocol version of the agent. | Upgrade the agent. See [Upgrade](/restichub/guides/upgrade/). |
| The list shows "Possible duplicate" | The computer enrolled again, and the old record stays. | On the newer record, select "Replace the old record". See [Possible duplicates](/restichub/guides/agent-certificates/#possible-duplicates). |
| The service exits with the code 78 | The agent has no usable identity, or restic is missing or differs from its record. The event log `ResticHub` (Linux: journald) names the cause. | For the identity, register the host again with `register --force`. For restic, run `install-service`. If the host cannot download restic, add `--restic-archive <ARCHIVE>`. |
| The service exits with the code 78 after an agent folder was opened in File Explorer | File Explorer gave an account access to `%ProgramData%\ResticHub`, and the service cannot remove that entry. The account is not a local administrator, or the folder holds a file that the agent refuses. | As an administrator, remove the entry with `icacls` and run `install-service` (see [Agent folders in File Explorer](/restichub/reference/agent-commands/#agent-folders-in-file-explorer)). |

For a message of an agent command, see [Agent commands and files](/restichub/reference/agent-commands/#troubleshooting).

## Jobs

Open the job, and read the reason under its title and the log of the failed step. [Status and reason codes](/restichub/reference/status-codes/) lists every reason.

| Symptom | Cause | Action |
|---|---|---|
| The job stays in "Waiting" | The host is offline, or another job holds the repository. | Check the host. See [Repository barriers](/restichub/concepts/job-lifecycle/#repository-barriers). |
| The job failed with `wrong_password` | The repository password or the access keys are wrong. | Correct the secrets on the page of the repository. |
| The job failed with `step_failed` | restic stopped with an error. | Read the log of the step. The last lines give the restic error. |
| A job on a local repository fails with `read-only file system`, `repository_path_denied` or `repository_read_only` | On Linux the service can write a local repository only below a repository root. The path is outside the roots, or the installed unit does not list the root yet. | Add the root to `/etc/restichub-agent-repository-roots.txt` and run `sudo /opt/restichub-agent/bin/restichub-agent install-service`. See [A local repository on Linux](/restichub/getting-started/add-a-repository/#a-local-repository-on-linux). |
| The job failed with `stalled` | restic made no progress for too long. | Check the backup source and the network to the repository. See [Stall timeout](/restichub/concepts/job-lifecycle/#stall-timeout). |
| The job failed with `agent_lost` | The host went offline during the job. | Check the host. The control plane marks a possible stale lock. |
| The job failed with `compile_failed` | The profile does not fit the host any more. | Open the profile and read the warnings. |
| The agent refuses a job with `server_scripts_disabled` | The job holds a script, and the host has no trust mode. | Turn on the trust mode on the host, or remove the script. See [Server scripts](/restichub/guides/scripts/). |
| The job is "Gave up" | The repository stayed locked after the last retry. | Find the job that holds the lock. See [Stale locks](/restichub/concepts/job-lifecycle/#stale-locks). |
| The job is "Expired" | The job waited longer than its acceptance window. | Check that the host is online at the time of the plan. See [Acceptance window](/restichub/concepts/job-lifecycle/#acceptance-window). |

## Restore

| Symptom | Cause | Action |
|---|---|---|
| The restore failed with `restore_target_denied` | The target path is outside the restore directories of the host. | Select a path inside a restore directory, or add a directory. See [Restore directories](/restichub/guides/restore/#restore-directories). |
| The snapshot tree does not open, `agent_capability_missing` | The agent is too old for the restore. | Upgrade the agent. |
| The list of snapshots is empty | The repository has no snapshot, or the filters hide them. | Select "Reset" above the list. |

## Alerts

| Symptom | Cause | Action |
|---|---|---|
| No email arrives | The SMTP server of the control plane is not set or refuses the message. | Select "Send test" on the channel. See [SMTP server](/restichub/guides/alerts/#smtp-server). |
| The control plane refuses a webhook channel | The address is outside the allow-list. | See [Webhook allow-list](/restichub/guides/alerts/#webhook-allow-list). |
| A link in a message opens a wrong address | The public origin of the control plane is not set. | See [Public origin](/restichub/guides/alerts/#public-origin). |
