---
title: Add a repository
description: Add an S3 or rest-server repository, set its maintenance host, and initialize it from a host.
---

A repository is the place where restic stores the snapshots. The control plane keeps the location of the repository and its secrets. The hosts connect to the repository directly, so the backup data does not go through the control plane.

## Before you start

- Enroll at least one host, and make sure that its status is "Online". See [Register an agent](/restichub/operations/register-an-agent/). This host becomes the maintenance host.
- Prepare a backend that the host can reach on the network:
  - An S3-compatible storage: a bucket, an access key and a secret key with the permission to read, write, list and delete objects in the bucket.
  - A [rest-server](https://github.com/restic/rest-server). Give each repository its own path, for example `https://rest.example.com:8000/fs-01`.
- Sign in as an Admin. Only an Admin creates a repository.

:::caution
ResticHub sends the REST server password without a user name. A rest-server with an `.htpasswd` file refuses the requests. In this version, run rest-server with `--no-auth` on a private network only, or use an S3-compatible storage.
:::

## The fields of the backend

| Backend kind | Field | Value |
|---|---|---|
| S3-compatible | "Endpoint" | The address of the storage with the scheme, for example `https://s3.example.com` |
| S3-compatible | "Bucket" | The bucket name, for example `backups` |
| S3-compatible | "Prefix" | Optional. A path in the bucket, for example `restic/fs-01` |
| S3-compatible | "Region" | Optional. The region of the bucket, for example `us-east-1` |
| S3-compatible | "S3 access key", "S3 secret key" | The credentials of the bucket |
| REST server | "Endpoint" | The address of the repository on the rest-server, for example `https://rest.example.com:8000/fs-01` |
| REST server | "REST server password" | Optional. See the caution above |

An "Endpoint" uses the scheme `https` or `http`. It cannot hold a user name, a query or a fragment.

## Create the repository

:::danger
Keep a copy of the repository password outside ResticHub. If you lose the control plane, restic cannot open the snapshots without the password. You cannot change the password after you save the repository.
:::

1. Open "Repositories".
2. Select "Add repository". The page "New repository" opens.
3. In the section "Main", enter a name in the field "Repository".
4. In the section "Location", select the "Backend kind": "S3-compatible" or "REST server".
5. Enter the fields of the backend from the table above.
6. In the section "Access", enter the "Repository password".
7. Enter the credentials of the backend.
8. In the section "Maintenance host", select the host.
9. Select "Save".

The repository appears in the list with the status "Not initialized". The control plane does not show a secret value again. You can only replace the S3 keys and the REST server password.

The sections "Maintenance" and "Check" hold the schedules of the maintenance jobs. Keep the defaults for the first repository. See [Repositories](/restichub/concepts/repositories/#maintenance).

## Initialize the repository

A new repository on an empty location needs an initialization. If the location already holds a restic repository with the same password, test the access instead.

1. In "Repositories", open the menu "…" of the repository row.
2. Select "Initialize". For an existing restic repository, select "Test access".
3. Make sure that the field "Host" shows the maintenance host. The control plane runs an initialization only on the maintenance host.
4. Select "Start".
5. Wait until the dialog shows the status "Succeeded" and the result "The repository is ready.".

The repository status changes to "Ready".

| Result | Meaning | Action |
|---|---|---|
| "The repository is ready." | Restic initialized the repository, or the access works. | None. |
| "The repository does not exist yet. Initialize it." | The test of the access found no repository at the location. | Select "Initialize". |
| "The repository password or the credentials are not correct." | Restic cannot open the repository. | Check the password and the credentials of the backend. |
| "The repository is not at this location on this host. Check the path, the mount and the host." | The host cannot reach the location. | Check the "Endpoint", the bucket and the network path from the host. |
| "The job expired before the host answered. Start it again." | The host did not answer in time. | Make sure that the host is online, then start the job again. |

## How the initialization works

The job holds a `test_access` step and, for an initialization, an `init` step. The agent runs `restic cat config` for the `test_access` step and `restic init` for the `init` step.

The agent runs `init` only when `test_access` exits with restic code 10. Restic code 10 means that the repository does not exist. Any other result stops the job, so the agent never initializes an existing repository a second time. A wrong password exits with code 12, and the job reports `config_error`.

The repository password and the backend credentials travel inside the signed job only. The agent passes them to restic in the environment. They never reach an argument, a log, the job journal or an error message.

A repository with a local path runs its repository jobs on the maintenance host only.

## Related

- [Repositories](/restichub/concepts/repositories/)
- [Profiles](/restichub/concepts/profiles/)
- [Register an agent](/restichub/operations/register-an-agent/)
