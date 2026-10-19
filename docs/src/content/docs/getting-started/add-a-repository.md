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
  - A local path: a directory on the disk of one host, for example `/mnt/backup/restic`. On Linux, allow the path on the host first. See [A local repository](#a-local-repository) and [A local repository on Linux](#a-local-repository-on-linux).
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
| Local path | "Path" | An absolute path on the disk of the repository host, for example `/var/spool/restichub/fs-01` |

One location holds one repository. The control plane refuses a repository with the location of another repository of your tenant, with the code `repository_location_taken`. The page names the repository that holds the location. A local location is the host and the path. An S3 location is the endpoint, the bucket and the prefix. A rest-server location is the URL.

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

## A local repository on Linux

The agent service on Linux runs in a systemd sandbox. The sandbox makes the file system read-only for the service, so restic cannot write a local repository at any path. The administrator of the host allows each place for local repositories in a file. The control plane and the jobs cannot change this file.

Do these steps on the repository host before you initialize the repository:

1. Write the parent directory of the repository on one line of the file `/etc/restichub-agent-repository-roots.txt`, for example `/srv/restichub` or `/var/spool/restichub`. Each line holds one absolute path. A line that starts with `#` is a comment. On Linux the agent ignores a line with a quotation mark, a backslash, `%`, a space or a control character. The agent logs a warning for these lines and for a line that is not an absolute path. The command `install-service` refuses such a line and names its number.
2. Make sure that root owns the file and that no other account can write it: `sudo chown root:root /etc/restichub-agent-repository-roots.txt` and `sudo chmod 0644 /etc/restichub-agent-repository-roots.txt`.
3. Run `sudo restichub-agent install-service`. The command adds each root to `ReadWritePaths` of the systemd unit and restarts the service. It creates a missing root with access for root only, and each missing parent with mode `0755`.

Run `install-service` again after each change of the file. The agent writes a warning to the log at each start for a root that the installed unit cannot write.

A root and each existing parent directory must belong to root. No group or other account may write there. The agent ignores a file or a root that does not follow this rule and logs a warning. `install-service` refuses such a root. Such an account could replace a directory with a link.

The agent refuses a local repository outside the roots before restic starts. The job fails with the reason `repository_path_denied`, and the screen names the file. The agent also refuses a path that holds or lies inside its own files.

A root cannot be `/`, a system directory (`/etc`, `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/boot`, `/proc`, `/sys`, `/dev`, `/run`, `/var/spool/cron`, `/var/lib/systemd`), a directory that holds one of them, `/home` or `/root` itself, a directory `/var/lib/restichub*` or a directory of the agent. Each root becomes writable for restic, so such a root would open the system files or the files of all accounts. Use a separate directory only for repositories as a root. The agent ignores such a root and logs a warning that names the line. `install-service` stops with a message.

The unit sets `ProtectHome=read-only`. The service reads a backup source below `/home`, but it writes below `/home`, `/root` and `/run/user` only in a root. A directory of a user account, for example `/home/<user>`, cannot hold a root, because it does not belong to root. Use a directory that root owns, for example `/srv/restichub`.

Do not put a local repository inside the directory of another local repository. The agent does not refuse it, but restic then finds the files of the second repository among the files of the first one.

The repository roots are not restore directories. A restore never writes into a repository root, also with the restore directory `/`. `install-service` refuses a restore directory and a repository root when one holds the other. See [Restore directories](/restichub/guides/restore/#restore-directories).

On Windows the service has no such sandbox. A local repository can use any path that the service account can write, and the agent does not read the repository roots file.

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
| "The host refused the local repository path. ..." | The path is outside the repository roots of a Linux host (`repository_path_denied`). | Add the root to `/etc/restichub-agent-repository-roots.txt` and run `install-service`. See [A local repository on Linux](#a-local-repository-on-linux). |
| "The agent cannot write to the local repository path. ..." | Restic reported `read-only file system` on the repository path (`repository_read_only`). The root is in the file, but the installed unit does not list it. | Run `sudo restichub-agent install-service`, then start the job again. |

## How the initialization works

The job holds a `test_access` step and, for an initialization, an `init` step. The agent runs `restic cat config` for the `test_access` step and `restic init` for the `init` step.

The agent runs `init` only when `test_access` exits with restic code 10. Restic code 10 means that the repository does not exist. In an initialization job, the job page shows this result of `test_access` as information: "No repository at this location. The next step creates it." Any other result stops the job, so the agent never initializes an existing repository a second time. A wrong password exits with code 12, and the job reports `config_error`.

The repository password and the backend credentials travel inside the signed job only. The agent passes them to restic in the environment. They never reach an argument, a log, the job journal or an error message.

## A local repository

A local path exists on the disk of one host only. Thus a local repository is bound to one host, the repository host. The repository host is also the maintenance host of the repository.

- You must select the repository host when you create the repository. The control plane refuses a local repository without a host.
- Every job of the repository runs on the repository host. This includes the backups of the plans, `forget`, `prune`, `check` and `unlock`. It also includes the initialization, the test of the access, the snapshot list, the tree pages and the restore.
- A plan that writes to the repository runs on the repository host only. The plan page offers only this host. See [Plans](/restichub/concepts/plans/#a-local-repository).
- A restore from the repository writes to the repository host only, because restic reads the repository on that host.
- Before the initialization, you can change the repository host. The control plane refuses the change while a plan of the repository runs on the old host. Move or delete the plan first.
- After the initialization, the repository host cannot change, like the path. The snapshots are on the disk of this host. The page shows the field locked with the reason, and the API refuses the change with the code `repository_host_locked`.

For an S3 or rest-server repository, each host reaches the repository on the network. A change of the maintenance host is a usual operation.

The replace of a host record moves the repository host to the new record, also after the initialization. The two records belong to the same machine, so the path stays on the same disk. See [Enroll a host again](/restichub/guides/agent-certificates/).

:::caution
Do not use a shared network disk (NFS or SMB) as a local repository for several hosts. ResticHub sends the jobs of a local repository to the repository host only, so the plans on the other hosts fail. For a repository that several hosts write to, use rest-server or SFTP.
:::

### The repository host is revoked

ResticHub cannot move an initialized local repository to another host. When the repository host is revoked, for example after a reinstall with a new machine identifier, do one of these procedures.

To keep the old snapshots on a new host:

1. Delete the repository in ResticHub. Move or delete its plans first. The restic data on the disk stays.
2. If the data moved to a new host, add it again as a new local repository on the new host. Use the same repository password.
3. Select "Test access". The test finds the existing repository and marks it initialized.

To keep the plans and the data at the same path:

1. Add a new local repository with the same path on the new host.
2. Open each plan of the old profile. Select the new repository and the new host in one change, then save.
3. On the new repository, select "Test access". The test finds the existing repository and marks it initialized.
4. Delete the old repository and the old profile.

The job history and the maintenance schedules of the old repository do not move to the new repository.

### Repositories saved before this rule

The control plane keeps a local repository that has no host, and a plan that runs on another host. It deletes nothing. The control plane refuses a job of such a plan with the reason `not_repository_host`. To find such a plan:

- Open "Jobs", select the tab "Failed" and filter by the repository. A failed job of such a plan shows the reason "The repository is local, and its path is on another host". The row names the plan.
- Open a plan. The host field shows an error when the plan runs on another host than its local repository.
- "Run now" of such a plan shows an error.

The log of the control plane also holds a warning for each such repository, with at most 20 plan identifiers. Only an administrator of the control plane reads this log.

To correct a repository:

1. Open the repository page.
2. In the section "Repository host", select the host that holds the path. A repository without a host accepts this change also when a plan runs on another host.
3. Open each plan that you found, and select the repository host.

## Delete a repository

Delete a repository from the menu "…" of the repository row in the list "Repositories". First move or delete each plan that writes to the repository, because the plans block the delete with the code `repository_in_use`. The control plane also refuses the delete while a job of the repository is active.

The delete removes the repository record and its stored secrets from ResticHub. It never deletes the restic data in the backend or on the disk. ResticHub sends no job to the host for the delete. Keep the repository password: ResticHub removes its copy, and you need the password to add the repository again.

## Related

- [Create a profile and a plan](/restichub/getting-started/profiles-and-plans/)
- [Repositories](/restichub/concepts/repositories/)
- [Profiles](/restichub/concepts/profiles/)
- [Register an agent](/restichub/getting-started/add-a-host/)
