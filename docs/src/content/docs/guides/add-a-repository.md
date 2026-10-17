---
title: Add a repository
description: Add an S3, rest-server or local repository, set its host, and initialize it from a host.
---

A repository is the place where restic stores the snapshots. The control plane keeps the location of the repository and its secrets. The hosts connect to the repository directly, so the backup data does not go through the control plane.

## Before you start

- Enroll at least one host, and make sure that its status is "Online". See [Register an agent](/restichub/getting-started/add-a-host/). This host becomes the maintenance host.
- Prepare a backend that the host can reach on the network:
  - An S3-compatible storage: a bucket, an access key and a secret key with the permission to read, write, list and delete objects in the bucket.
  - A [rest-server](https://github.com/restic/rest-server). Give each repository its own path, for example `https://rest.example.com:8000/fs-01`. If the rest-server uses an `.htpasswd` file, prepare the user name and the password of one account in the file.
  - A local path: a directory on the disk of one host, for example `/mnt/backup/restic`. See [A local repository](#a-local-repository).
- Sign in as an Admin. Only an Admin creates a repository.

## The fields of the backend

| Backend kind | Field | Value |
|---|---|---|
| S3-compatible | "Endpoint" | The address of the storage with the scheme, for example `https://s3.example.com` |
| S3-compatible | "Bucket" | The bucket name, for example `backups` |
| S3-compatible | "Prefix" | Optional. A path in the bucket, for example `restic/fs-01` |
| S3-compatible | "Region" | Optional. The region of the bucket, for example `us-east-1` |
| S3-compatible | "S3 access key", "S3 secret key" | The credentials of the bucket |
| REST server | "Endpoint" | The address of the repository on the rest-server, for example `https://rest.example.com:8000/fs-01` |
| REST server | "User name" | Optional. The user name of the account in the `.htpasswd` file of the rest-server |
| REST server | "REST server password" | Optional. The password of the same account |
| Local path | "Path" | An absolute path on the disk of the repository host, for example `/mnt/backup/restic` |

An "Endpoint" uses the scheme `https` or `http`. It cannot hold a user name, a password, a query or a fragment. Enter the credentials of the rest-server in the fields "User name" and "REST server password" in the section "Access".

The user name is not a secret: the repository page shows it, and you can change it at any time. The password is a secret. The agent gives both values to restic in the environment variables `RESTIC_REST_USERNAME` and `RESTIC_REST_PASSWORD`, never in the command line. A rest-server with `--no-auth` needs no credentials.

## Create the repository

:::danger
Keep a copy of the repository password outside ResticHub. If you lose the control plane, restic cannot open the snapshots without the password. You cannot change the password after you save the repository.
:::

1. Open "Repositories".
2. Select "Add repository". The page "New repository" opens.
3. In the section "Main", enter a name in the field "Repository".
4. In the section "Location", select the "Backend kind": "S3-compatible", "REST server" or "Local path".
5. Enter the fields of the backend from the table above.
6. In the section "Access", enter the "Repository password".
7. Enter the credentials of the backend.
8. In the section "Maintenance host", select the host. For a local repository, the section is "Repository host", and the host is necessary.
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

## A local repository

A local path exists on the disk of one host only. Thus a local repository is bound to one host, the repository host. The repository host is also the maintenance host of the repository.

- You must select the repository host when you create the repository. The control plane refuses a local repository without a host.
- Every job of the repository runs on the repository host. This includes the backups of the plans, `forget`, `prune`, `check` and `unlock`. It also includes the initialization, the test of the access, the snapshot list, the tree pages and the restore.
- A plan of a profile that writes to the repository runs on the repository host only. The plan page offers only this host. See [Plans](/restichub/concepts/plans/#a-local-repository).
- A restore from the repository writes to the repository host only, because restic reads the repository on that host.
- Before the initialization, you can change the repository host. The control plane refuses the change while a plan of the repository runs on the old host. Move or delete the plan first.
- After the initialization, the repository host cannot change, like the path. The snapshots are on the disk of this host. The page shows the field locked with the reason, and the API refuses the change with the code `repository_host_locked`.

For an S3 or rest-server repository, each host reaches the repository on the network. A change of the maintenance host is a usual operation.

### Repositories saved before this rule

The control plane keeps a local repository that has no host, and a plan that runs on another host. It deletes nothing. At the start, the control plane writes a warning to its log for each such repository, with the identifiers of the plans. The control plane refuses a job of such a plan with the reason `not_repository_host`. To correct a repository:

1. Open the repository page.
2. In the section "Repository host", select the host that holds the path. A repository without a host accepts this change also when a plan runs on another host.
3. Open each plan that the log names, and select the repository host.

## Related

- [Repositories](/restichub/concepts/repositories/)
- [Profiles](/restichub/concepts/profiles/)
- [Register an agent](/restichub/getting-started/add-a-host/)
