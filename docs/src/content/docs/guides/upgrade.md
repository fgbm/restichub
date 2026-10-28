---
title: Upgrade
description: Upgrade the control plane and the agents on Linux and Windows to a new release, for example from 0.1.0-rc.4.
---

Upgrade the control plane first. Then upgrade the agents. The control plane accepts an agent of the previous minor protocol version, so the agents keep working while you upgrade them one by one. See [Agent and control plane versions](/restichub/reference/agent-version-compatibility/).

## Before you start

1. Read the section "Upgrade notes" of each release between your version and the new version on the [release page](https://github.com/fgbm/restichub/releases).
2. Examine the directory `/opt/restichub`. If it holds the directory `.git`, the server was installed with `git clone`. Then do the steps in [Move a server that was installed with git](#move-a-server-that-was-installed-with-git) instead of the upgrade steps.

## Upgrade the control plane

The script `install.sh` of the new release upgrades the server. It verifies the release with the same checks as the installation. Then it makes a backup, replaces the release files and starts the new image.

:::caution
Keep the backup of the script. The migrations of a new release change the database. The backup is the only way back to the old release.
:::

1. Open a `tmux` or `screen` session on the server. If the SSH connection stops, the session keeps the script running.
2. Set the version of the new release:

   ```sh
   VERSION=<version>
   ```

3. Run the script of the new release:

   ```sh
   curl -fsSL "https://github.com/fgbm/restichub/releases/download/v${VERSION}/install.sh" | sudo bash -s -- --upgrade
   ```

   The script prints `The upgrade to <version> is complete`, the schema version before and after the upgrade, and the backup directory. To verify the script before you run it, use the steps in [Install with a verified download](/restichub/getting-started/install-control-plane/#install-with-a-verified-download) with the option `--upgrade`.

4. Copy the database dump from the backup directory to another host.

The script does these steps:

1. It verifies `cosign`, the release archive and the image of the new release.
2. It stops when another run of `install.sh` holds the lock `/run/restichub-install.lock`. It also stops when an earlier run failed and left the file `/etc/restichub/INSTALL-FAILED`.
3. It stops when `/opt/restichub` holds an override file, for example `compose.override.yaml`, or when the environment file sets a `COMPOSE_` variable. With such a file or variable, the new stack can start on another database.
4. It records the database of the running stack: the volume `restichub_pgdata`, the system identifier of PostgreSQL, the schema version and the number of users.
5. It compares the KEK file that the running control plane uses with `/etc/restichub/secrets/restichub_kek`. It compares only the SHA-256 values.
6. It makes the new directory `/var/backups/restichub/<time>` with the mode `700`. The option `--backup-dir` sets another parent directory. The parent must be an absolute path outside `/opt/restichub` and `/etc/restichub`. The script does not change the mode of a parent directory that exists.
7. It checks the free space. The backup directory and the Docker data directory must each have two times the size of the database.
8. It copies the environment file and the old release files to the backup directory. It makes a database dump there and reads all the data of the dump with `pg_restore -f /dev/null`.
9. It writes the commands that go back to the file `ROLLBACK.txt` in the backup directory. Then it replaces the files in `/opt/restichub` and sets the new image in `RESTICHUB_IMAGE`.
10. It starts PostgreSQL alone and compares the volume and the system identifier with the recorded values. If they differ, it stops the new stack before a migration runs.
11. It starts the stack. The `migrate` service applies the new migrations before the control plane starts.
12. It waits until the control plane is healthy and compares the running image with the verified image. If the database had users and the control plane prints a new setup token, the script stops. Such a control plane does not use the old database.
13. If the `Caddyfile` changed, it starts a new `caddy` container. A running container keeps the old file.

If a step after the backup fails, the script prints the commands that start the old release again. It also writes them to `ROLLBACK.txt`. The text tells if a migration ran:

- `No migration ran`: the commands do not restore the dump.
- `The schema moved <old> -> <new>`: the commands restore the dump first. The old release cannot use the new schema.

The script also writes the file `/etc/restichub/INSTALL-FAILED`. A new run stops until you go back and remove the file. The option `--force-after-failure` starts a new run with the file. Use the option only when you know the cause of the failure. The new run makes its backup from the state that the failed run left.

The agents connect again by themselves. See [Go back to the old release](#go-back-to-the-old-release).

### Old backups

The script does not delete old backups. Each run adds one directory to `/var/backups/restichub`. To remove an old backup, do these steps:

1. Make sure that the new release works and that a copy of the newest dump is on another host.
2. Show the backup directories:

   ```sh
   sudo ls /var/backups/restichub
   ```

3. Remove a directory that you do not need:

   ```sh
   sudo rm -rf /var/backups/restichub/<time>
   ```

### Upgrade by hand

Use these steps only when you cannot run the script. They change only the image. If the upgrade notes name a change of the release files, use the script.

1. Verify the new release and read its image reference with the steps in [Verify a release](/restichub/operations/verify-a-release/).
2. Make a database dump. See [Back up the database](/restichub/operations/back-up-the-control-plane/#back-up-the-database).
3. Open a root shell (`sudo -i`), and go to the directory of the stack:

   ```sh
   cd /opt/restichub
   ```

4. Copy the environment file:

   ```sh
   cp -p /etc/restichub/.env "/etc/restichub/.env.$(date -u +%Y%m%dT%H%M%SZ)"
   ```

5. Set the verified image reference in the shell:

   ```sh
   NEW_IMAGE=ghcr.io/fgbm/restichub-cp@sha256:<digest>
   ```

6. Write the reference to the environment file:

   ```sh
   sed -i "s|^RESTICHUB_IMAGE=.*|RESTICHUB_IMAGE=${NEW_IMAGE}|" /etc/restichub/.env
   ```

7. Pull the image and restart the stack:

   ```sh
   docker compose pull
   docker compose up -d
   ```

8. Wait until `docker compose ps cp` shows `(healthy)`.

### Check the new version

1. In a root shell in `/opt/restichub`, show the image of the running control plane:

   ```sh
   docker inspect --format '{{.Config.Image}}' "$(docker compose ps -q cp)"
   ```

   The command prints the image reference of the new release.

2. Make sure that `curl -fsS https://restichub.example.com/healthz` prints `ok`.
3. Sign in to the web interface. Open the account menu, and select "About ResticHub". The field "Version" shows the new version.
4. For a check from a script, read `GET /api/v1/about` with an API token. The field `version` holds the new version.
5. Within a few minutes, the screen "Hosts" shows the hosts "Online" again.
6. An agent with an older package shows the text `agent 0.1.0-rc.4, 0.1.0 is available` in its row.

## Move a server that was installed with git

An earlier version of this guide installed the control plane with `git clone` into `/opt/restichub`. Such a server keeps the environment file and the KEK in `/opt/restichub/deploy/reference`. A `git clean` or a new clone in that directory deletes the KEK. The option `--migrate-from-git` of `install.sh` moves the server to the layout of the script and upgrades it to the version of the script.

:::caution
The script copies the KEK, and it never deletes the KEK of the clone. Delete the clone only after you put a copy of the KEK outside the server.
:::

### Before you move a production server

Do these checks before you move a production server:

1. Make sure that `/opt/restichub/deploy/reference` holds no override file: `compose.override.yaml`, `compose.override.yml`, `docker-compose.override.yaml` or `docker-compose.override.yml`. The script stops when it finds one.
2. Make sure that the project of the stack is `restichub`:

   ```sh
   sudo docker compose ls
   ```

3. Make sure that the database is in the volume `restichub_pgdata`:

   ```sh
   sudo docker inspect --format '{{range .Mounts}}{{.Name}} {{.Destination}}{{println}}{{end}}' restichub-postgres-1
   ```

   The output has the line `restichub_pgdata /var/lib/postgresql/data`.

4. Copy the KEK and the environment file of the clone to a place outside the server.
5. Make sure that `/var/backups` has free space for two times the size of the database.
6. Make sure that `/var/tmp` does not have the mount option `noexec`. The command `findmnt -T /var/tmp` shows the options. The script runs `cosign` from this directory.
7. Make sure that `git` is on the server. The script compares the files of `deploy/reference` with the commit of the clone.
8. Show the tag of the clone with `git -C /opt/restichub describe --tags`. Use a release with the same version or a later version.
9. Use the steps in [Install with a verified download](/restichub/getting-started/install-control-plane/#install-with-a-verified-download), not `curl … | sudo bash`.
10. Open a `tmux` or `screen` session on the server.

### Move the server

1. Make sure that the stack of the clone runs. The script makes the database dump from the running PostgreSQL container and compares the KEK of the running control plane.
2. Set the version of the new release:

   ```sh
   VERSION=<version>
   ```

3. Run the verified script of the new release with the option `--migrate-from-git`:

   ```sh
   sudo bash install.sh --migrate-from-git /opt/restichub
   ```

   The script prints `The server uses the files of this script now`, the schema version before and after the move, and the first 8 characters of the SHA-256 of the KEK.

4. Do the steps 1 to 7 in [Store the KEK outside the server](/restichub/getting-started/install-control-plane/#store-the-kek-outside-the-server). After a move, the script does not wait for the word `saved`, because it uses the KEK of the clone.
5. Do the steps in [Check the new version](#check-the-new-version).
6. Wait until the screen "Hosts" shows each host "Online". Then delete the old clone. The script prints the command with the directory, for example:

   ```sh
   sudo rm -rf /opt/restichub.git-backup-<time>
   ```

The script does these steps:

1. It verifies `cosign`, the release archive and the image. Until then it changes nothing.
2. It stops when the clone has an override file, a `COMPOSE_` variable in its environment file, or a change of a file of `deploy/reference` that `git` tracks. It also stops when the KEK file or the environment file of the clone is a symbolic link.
3. It records the database of the running stack, as the upgrade does. The running stack must come from `deploy/reference/compose.yaml` of the clone, and the database must be in the volume `restichub_pgdata`.
4. It copies `deploy/reference/.env` and `deploy/reference/secrets/restichub_kek` of the clone to `/etc/restichub`. It gives the environment file the mode `600` and the owner root, and the KEK the mode `400` and the owner uid 10001. It compares the SHA-256 of each copy with its source.
5. It makes a database dump in `/var/backups/restichub/<time>` and reads all the data of the dump with `pg_restore -f /dev/null`.
6. It writes `ROLLBACK.txt` to the backup directory and renames the clone to `/opt/restichub.git-backup-<time>`.
7. It writes the release files to `/opt/restichub` and starts the stack with the same project name `restichub`, as the upgrade does. The stack uses the same volumes, so the data stays.

If a step after the rename fails, the script prints the commands that start the stack of the clone again, and writes them to `ROLLBACK.txt`. See [Go back after a move from git](#go-back-after-a-move-from-git).

## Upgrade an agent on Linux

You do not register the host again. The identity, the restic record and the trust mode stay.

1. On the host, download the new agent package:

   ```sh
   curl -LO https://github.com/fgbm/restichub/releases/download/v<version>/restichub-agent-<version>-linux-amd64.tar.gz
   ```

2. Unpack it in an empty directory:

   ```sh
   mkdir agent-new && tar -xzf restichub-agent-<version>-linux-amd64.tar.gz -C agent-new
   ```

3. Install the new agent:

   ```sh
   sudo ./agent-new/restichub-agent install-service
   ```

The command stops the service, copies the new program, starts the service and waits for the connection. Use the package `linux-arm64` on an ARM host. The command also refuses while a job runs in the service, as on Windows: it prints the message `cannot stop the service`. Wait for the job to end, then run the command again.

## Upgrade an agent on Windows

1. Download `restichub-agent-<version>-windows-amd64.zip` from the release page.
2. Unpack the archive in an empty folder, for example in "Downloads".
3. Start PowerShell with "Run as administrator", and go to the folder.
4. Install the new agent:

   ```powershell
   .\restichub-agent.exe install-service
   ```

The command stops the service, copies the new program to `%ProgramFiles%\ResticHub\Agent` and starts the service. It also repairs the access control of `%ProgramData%\ResticHub` that an older agent left. If an older agent or a manual copy left `%ProgramFiles%\ResticHub` open, the command makes it again. If an administrator opened `%ProgramData%\ResticHub\Agent` in File Explorer, the command removes the entry that File Explorer added for that account and prints a warning. You do not register the host again for this entry (see [Agent folders in File Explorer](/restichub/reference/agent-commands/#agent-folders-in-file-explorer)). If the command stops with a message, see [Agent commands and files](/restichub/reference/agent-commands/#upgrade-the-agent).

The command refuses while a job runs in the service. It prints the message `cannot stop the service`, and the service continues the job. Wait for the job to end, then run the command again. If the agent cannot read the job journal, the message also names the journal. An entry that a crash left in the journal does not refuse the command. The service marks it at its start, and only a job of the running service refuses. If you stop the service, the agent cancels the running job and waits for it, so restic removes its repository lock before the process ends. A restic that does not answer the stop signal keeps its lock until the lock expires.

The command cannot move an open `%ProgramFiles%\ResticHub` while a process keeps a file or a directory in it open. Do not go to a folder in `%ProgramFiles%\ResticHub` in PowerShell. If another process keeps the folder open, the command repairs the folder in place and starts the service. Then it prints a warning that names that process. That process can add a file to the folder until the restart. Thus the service starts only by hand until you do these steps:

1. Close that process, or remove it from the programs that start when you sign in.
2. Restart the computer with "Restart", not "Shut down". After the restart, the service does not start automatically.
3. Run `install-service` again as Administrator.
4. Make sure that the output does not show a line that starts with `repaired` or `moved`. This run sets the automatic start of the service again.

The command removes each file that the agent does not know from an open folder, as it does when it moves the folder. The output shows one line for each removed file.

## Check the agents

1. Open "Hosts".
2. Read the second line of each row. A current agent shows only its version, for example `agent 0.1.0`.
3. If a host shows "Agent too old", upgrade the agent of that host.

## Go back to the old release

If the new control plane does not start, go back to the old release. When `install.sh --upgrade` fails, it prints these commands with the directory of its backup, and writes them to `ROLLBACK.txt` in that directory.

:::danger
The restore of the dump deletes the database of the new release. The jobs and the changes after the backup are lost. A host that enrolled after the backup must enroll again.
:::

1. Read `/var/backups/restichub/<time>/ROLLBACK.txt`. It tells if a migration ran.
2. Open a root shell (`sudo -i`), and stop the stack:

   ```sh
   cd /opt/restichub && docker compose down
   ```

3. Put back the environment file of the old release. It holds the old image reference:

   ```sh
   cp -p /var/backups/restichub/<time>/restichub.env /etc/restichub/.env
   ```

4. Put back the old release files:

   ```sh
   cd / && rm -rf /opt/restichub && cp -a /var/backups/restichub/<time>/release /opt/restichub
   ```

5. If the text says `No migration ran`, go to step 7.
6. Restore the dump. Do not run `docker compose down -v`: it also removes the volumes of Caddy.

   ```sh
   cd /opt/restichub
   docker volume rm restichub_pgdata
   docker compose up -d --wait postgres
   docker compose run --rm -T db-bootstrap
   docker compose exec -T postgres pg_restore -U restichub -d restichub --exit-on-error < /var/backups/restichub/<time>/restichub-<time>.dump
   ```

7. Start the stack:

   ```sh
   cd /opt/restichub && docker compose up -d
   ```

8. Remove the marker of the failed run:

   ```sh
   rm -f /etc/restichub/INSTALL-FAILED
   ```

## Go back after a move from git

When `install.sh --migrate-from-git` fails after it renamed the clone, it prints these commands and writes them to `ROLLBACK.txt` in the backup directory. The backup directory holds the dump. The clone is in `/opt/restichub.git-backup-<time>`.

1. Read `/var/backups/restichub/<time>/ROLLBACK.txt`. It tells if a migration ran.
2. Open a root shell (`sudo -i`), and stop the new stack:

   ```sh
   cd /opt/restichub && docker compose down
   ```

3. Remove the release files and put back the clone:

   ```sh
   cd / && rm -rf /opt/restichub
   mv /opt/restichub.git-backup-<time> /opt/restichub
   ```

4. If the text says `No migration ran`, go to step 6.
5. Restore the dump in the stack of the clone:

   ```sh
   cd /opt/restichub/deploy/reference
   docker volume rm restichub_pgdata
   docker compose up -d --wait postgres
   docker compose run --rm -T db-bootstrap
   docker compose exec -T postgres pg_restore -U restichub -d restichub --exit-on-error < /var/backups/restichub/<time>/restichub-<time>.dump
   ```

6. Start the stack of the clone:

   ```sh
   cd /opt/restichub/deploy/reference && docker compose up -d
   ```

7. Remove the marker of the failed run:

   ```sh
   rm -f /etc/restichub/INSTALL-FAILED
   ```

To move the server later, run the script again. The script accepts the copies in `/etc/restichub` of the failed run. It compares them with the files of the clone, without the line `RESTICHUB_IMAGE`.

Upgrade the agents only after the new control plane works.

## Upgrade notes since 0.1.0-rc.4

### Control plane

- Make the backup of the database as the PostgreSQL superuser of the stack, as [Back up the control plane](/restichub/operations/back-up-the-control-plane/#back-up-the-database) shows. The command of the 0.1.0-rc.4 documentation, `pg_dump -U restichub_owner`, does not work. The tables have row-level security, so the dump stops with `query would be affected by row-level security policy`.
- Ten database migrations (0049 to 0058) run at the first start. The migrations 0049 to 0055 add the indexes and the fields of the lists, and the password reset of a platform account. They also add the user name of a rest-server and the stream state of a host. A blank email of an account becomes no email. The migration 0055 replaces the unique keys of the names. The migrations 0056 and 0057 move the repository to the plan and add the profile parameters. The migration 0058 adds the priority of a profile. See the sections below.
- Names may now contain any text. The name of a profile, a plan or an alert channel is 1 to 100 characters, with any letters, spaces and emoji. It is unique in the tenant, and the letter case does not matter. The names that you made before stay valid.
- The comparison of names uses the function `lower()` of PostgreSQL, so it follows the `LC_CTYPE` of the database. The reference stack uses `en_US.utf8`. With the locale `C`, only the ASCII letters fold. Then the names "Ночной" and "НОЧНОЙ" can both exist.
- One location holds one repository. The control plane refuses a new or changed repository with the location of another repository of the tenant. It does not delete a duplicate that exists. It logs a warning, and the list of repositories shows the mark "Shared location". Delete the duplicate records in ResticHub. A delete never deletes restic data. See [Add a repository](/restichub/getting-started/add-a-repository/).
- A local repository is bound to its host. The control plane refuses a job of a local repository on another host with the reason `not_repository_host`. If several hosts used one shared network disk (NFS or SMB) as a local repository, move this repository to rest-server or SFTP. To find and correct the old plans, see [Repositories saved before this rule](/restichub/getting-started/add-a-repository/#repositories-saved-before-this-rule).
- For each REST server repository with a password, enter the user name of the account in the field "User name". See [User name of a rest-server](/restichub/reference/agent-version-compatibility/#user-name-of-a-rest-server).
- Scripts and other API clients get new error codes. A refused delete returns a code with the reason instead of `conflict`, for example `repository_in_use` or `profile_in_use`. A new local repository needs `maintenance_agent_id`. A taken user name or email returns `username_taken` or `email_taken`. See [Status and reason codes](/restichub/reference/status-codes/).

### The repository in the plan

- The plan holds the repository, and a profile holds none. The migration 0056 moves the repository of each profile to its plans. A profile without a plan loses its repository, and the next plan of the profile selects one. The plan page shows no notice of the move.
- A plan of a local repository on another host is refused with the field `agent_id` and the reason `not_repository_host`. The control plane also refuses the delete of a repository that a plan uses, with the code `repository_in_use` and the plans in `blockers`.
- The plans of a repository share its retention. The retention task gives one `forget` step for each distinct profile of the plans of the repository. The step takes the facts of an agent from the plans of this profile in this repository only.
- `POST /api/v1/profiles` and `PATCH /api/v1/profiles/{id}` accept no `repository_id`. A request with the field is refused as an unknown field. The profile view holds no `repository_id`.
- `POST /api/v1/plans` and `PATCH /api/v1/plans/{id}` require `repository_id`. A missing or an empty value gives `400` with the field `repository_id` and the reason `empty`.
- The route `POST /api/v1/profiles/{id}/preview` is removed. Use `POST /api/v1/profiles/preview` with `agent_id` and an optional `repository_id`. A person without the profile write right also sends `profile_id`, and the server compiles the stored profile.

### Profile parameters

- A profile declares parameters. A host holds the values, and a plan can override a value. The profile editor has the section "Parameters", the host page the section "Parameter values", and the plan page the section "Profile parameters". The migration 0057 adds the declarations and the values. An existing profile declares no parameter, so no plan changes.
- A script variable has a new kind, "Profile parameter". The value of a text parameter reaches the agent as a masked value. An agent of an older package receives it as a secret value and masks it in the same way. See [Agent and control plane versions](/restichub/reference/agent-version-compatibility/#masked-script-values).
- Only an Admin sets, changes or removes a secret parameter value. An Operator sets the text values. Only an Admin deletes a plan that holds a secret value.
- A save that leaves a plan without a required parameter gives `400` with `parameter_missing` or `parameter_missing_in_plans`. See [Status and reason codes](/restichub/reference/status-codes/#refused-parameter-values).

### Profile priority

- A profile has the option `options.priority` with the values `normal` and `low`. The migration 0058 gives each existing profile the value `normal`, so the priority of a running job does not change.
- The value `low` applies to the `backup` step and to the `forget` step of the profile. The `prune` and `check` steps keep the normal priority.
- `PATCH /api/v1/profiles/{id}` replaces the options as a whole. A request without `options.priority` sets the value `normal`. An API client that sends the options of a profile sends the priority too.

### All agents

- The service stops with the exit code 78 when its identity, its program or its restic is not safe. A restic that is missing or that differs from its record also stops the service. The service checks the identity again at each new connection. It does not restart after the exit code 78. `status` shows the cause. For a missing restic, run `install-service --restic-archive <ARCHIVE>`.

### Windows agents

- `install-service` and `register` repair the access control of `%ProgramData%\ResticHub`. Read the output: it prints one line for each top-level directory, and one line for each removal and each warning. Add `--verbose` to print each repaired item. An open `%ProgramFiles%\ResticHub` is moved aside and made again, so run the command from the unpacked package. If a process keeps that folder open, the command repairs it in place and prints a warning. Until you restart the computer and run the command again, the service starts only by hand.
- To restart the computer, select "Restart", not "Shut down". With the fast startup of Windows, "Shut down" keeps the system session, and the agent does not see a restart.
- The Windows service restarts after each failure, after 5, 30 and then 60 seconds. It does not restart after the exit code 78, as on Linux.
- A Windows service with an identity that another account could change does not start. It writes the cause to the event log `ResticHub`, and `status` shows it. Before this release such a service connected, and each job failed with "restic did not start".
- The Windows service writes its log to rotated files in `%ProgramData%\ResticHub\Agent-logs` and to the event log `ResticHub`. See [Agent logs](/restichub/reference/agent-logs/).
- Before a restore into a directory that is not empty, the agent checks the paths that the restore writes. A link that leads out of the target stops the restore step. A file with more than one name also stops it. Remove the link, or restore into an empty directory. See [Restore directories](/restichub/guides/restore/#restore-directories).

### Windows hosts that need a new registration

The releases 0.1.0-rc.2, 0.1.0-rc.3 and 0.1.0-rc.4 make each missing level of `%ProgramData%\ResticHub` and `%ProgramFiles%\ResticHub` with the access control for SYSTEM and Administrators only. They do not make a directory with the inherited access. An open access has two causes. An agent build before 0.1.0-rc.2 made `%ProgramData%\ResticHub` with the access of `%ProgramData%`. A person made a folder by hand, for example for a manual copy of the agent to `%ProgramFiles%\ResticHub\Agent`.

On such a host, do these steps one time:

1. Run `install-service` of the new package as Administrator.
2. If the command says `the identity of this host could be changed by another account`, create a token with "Re-register" on the host page.
3. Run `.\restichub-agent.exe register --force --url <URL> --token <TOKEN>` in the folder of the package, as Administrator. The command repairs all the directories first. It removes the old identity only after all the checks pass.
4. If a command printed `repaired the access control of ... under C:\ProgramData\ResticHub...`, restart the computer. A program that opened that directory before the repair keeps the access of its handle until the restart. With that handle it can put back its access entry. The service then refuses the identity and stops with the exit code 78.
5. After the restart, run `install-service` again. Its output must not show a line that starts with `repaired`. If it shows such a line, a program with administrator rights changes the rights of the directory. An example is a management tool or a sync tool. Find that program and stop it. Then do the steps 4 and 5 again.

A host that you registered first with 0.1.0-rc.2 or a later release on a clean computer does not need these steps. `install-service` alone is sufficient there.

### Linux agents

`install-service` writes the systemd unit again and removes a change that you made by hand. The new unit has a system call filter (`SystemCallFilter=@system-service`) and hides the other processes (`ProtectProc=invisible`). A system call outside the filter fails with the error `EPERM`. Thus a server script cannot use a debug tool such as `strace` or `perf`. See [Server scripts](/restichub/guides/scripts/).

The new unit also does not set `RestrictSUIDSGID`. A restore of a file with a setuid or setgid mode failed on a host with the older unit. Such a restore wrote the file without the setuid or setgid bit. Do the restore again after you run `install-service`. In "Existing files", select "Replace every file" or "Replace a file that differs", or restore into an empty directory. With "Keep every existing file" or "Replace a file that is older than the snapshot", restic skips the file that the failed restore left. The job then ends with the status `succeeded`, but the file has no setuid or setgid bit.

The new unit also keeps the `CAP_SETFCAP` capability. A restore of a file with a file capability, for example `ping`, failed on a host with the older unit. The job ended with the status `failed`. Run `install-service` from the new package. Then do the restore again. In "Existing files", select "Replace every file" or "Replace a file that differs", or restore into an empty directory. With "Keep every existing file" or "Replace a file that is older than the snapshot", restic skips the file that the failed restore left. The job then ends with the status `succeeded`, but the file has no file capability.

### Local repositories on Linux

The new agent on Linux writes a local repository only below a repository root. The systemd unit now also sets `ProtectHome=read-only`, so `/home`, `/root` and `/run/user` are read-only for the service. An older unit left these directories writable. Thus a local repository below `/home` or `/root` worked before, and after the upgrade the agent rejects its jobs with `repository_path_denied`. `install-service` also writes the unit again and removes a change that you made by hand.

Do these steps on each Linux host that keeps a local repository:

1. Open "Repositories" and find each repository with the backend kind "Local path" on this host.
2. Check the path of each repository. If another account owns a parent directory, for example `/home/<user>`, move the repository to a directory that root owns. The agent refuses such a root, because that account could replace a directory with a link. For example:

   ```sh
   sudo systemctl stop restichub-agent
   sudo install -d -o root -g root -m 0755 /srv/restichub
   sudo mv /home/<user>/backup /srv/restichub/fs-01
   sudo chown -R root:root /srv/restichub/fs-01
   ```

3. Write each parent directory of the repositories on one line of the file `/etc/restichub-agent-repository-roots.txt`, for example `/srv/restichub`. Root must own the file, and no other account may write it: `sudo chmod 0644 /etc/restichub-agent-repository-roots.txt`.
4. Run `sudo /opt/restichub-agent/bin/restichub-agent install-service`. The command adds each root to `ReadWritePaths` and starts the service.
5. If you moved a repository, open it in "Repositories" and enter the new path in the field "Path".
6. Start "Test access" for each local repository. The result must be "The repository is ready.".

A repository root cannot be `/`, a system directory such as `/etc` or `/usr`, or a directory of the agent. A restore directory and a repository root cannot hold each other, except the restore directory `/`. `install-service` stops with a message that names the line.

With the restore directory `/`, an in-place restore below `/home` or `/root` now fails with `read-only file system`. A restore directory `/home` or `/root` does not change this, and `install-service` now refuses such a directory. Use a directory below it that root owns. Restore to another directory, for example `/var/lib/restichub-restore`, and copy the files to their place. See [Restore directories](/restichub/guides/restore/#restore-directories).
