---
title: Upgrade
description: Upgrade the control plane and the agents on Linux and Windows to a new release, for example from 0.1.0-rc.4.
---

Upgrade the control plane first. Then upgrade the agents. The control plane accepts an agent of the previous minor protocol version, so the agents keep working while you upgrade them one by one. See [Agent and control plane versions](/restichub/reference/agent-version-compatibility/).

## Before you start

1. Read the section "Upgrade notes" of each release between your version and the new version on the [release page](https://github.com/fgbm/restichub/releases).
2. Verify the new release and read its image digest. See [Verify a release](/restichub/operations/verify-a-release/).
3. Make a backup of the control plane. See [Back up the control plane](/restichub/operations/back-up-the-control-plane/).

## Upgrade the control plane

:::caution
Do not skip the backup. The migrations of a new release change the database. The backup is the only way back to the old release.
:::

1. Go to the checkout:

   ```sh
   cd /opt/restichub
   ```

2. Get the new release tag, for example `v0.1.0`:

   ```sh
   git fetch --depth 1 origin tag v0.1.0
   ```

3. Check out the tag:

   ```sh
   git checkout v0.1.0
   ```

4. In the file `deploy/reference/.env`, set `RESTICHUB_IMAGE` to the new image reference with the digest.
5. Go to the directory of the reference stack:

   ```sh
   cd deploy/reference
   ```

6. Pull the image and restart the stack:

   ```sh
   docker compose pull
   docker compose up -d
   ```

7. Wait until `docker compose ps cp` shows `(healthy)`.

The `migrate` service applies the new migrations before the control plane starts. The agents connect again by themselves.

Check the result:

- `curl -fsS https://restichub.example.com/healthz` prints `ok`.
- The screen "Hosts" shows the hosts "Online" again within a few minutes.
- An agent with an older package shows the text `agent 0.1.0-rc.4, 0.1.0 is available` in its row.

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

If the new control plane does not start, go back to the old release:

:::danger
The step 4 deletes the database of the new release. The jobs and the changes after the backup are lost. A host that enrolled after the backup must enroll again.
:::

1. Make sure that the backup of the step "Before you start" is available.
2. Stop the stack with `docker compose down`.
3. Check out the old release tag, and set the old image reference in `.env`.
4. Remove the volume of the database:

   ```sh
   docker volume rm restichub_pgdata
   ```

   The project name is `restichub`, as the file `compose.yaml` gives it. A stack with another project name has another volume, `<project>_pgdata`. Do not run `docker compose down -v`: it also removes the volumes of Caddy.
5. Restore the database from the backup. See [Restore on a new host](/restichub/operations/back-up-the-control-plane/#restore-on-a-new-host). The volume is empty now, so the restore runs as on a new host.
6. Start the stack with `docker compose up -d`.

Upgrade the agents only after the new control plane works.

## Upgrade notes since 0.1.0-rc.4

### Control plane

- Make the backup of the database as the PostgreSQL superuser of the stack, as [Back up the control plane](/restichub/operations/back-up-the-control-plane/#back-up-the-database) shows. The command of the 0.1.0-rc.4 documentation, `pg_dump -U restichub_owner`, does not work. The tables have row-level security, so the dump stops with `query would be affected by row-level security policy`.
- Nine database migrations (0049 to 0057) run at the first start. The migrations 0049 to 0055 add the indexes and the fields of the lists, and the password reset of a platform account. They also add the user name of a rest-server and the stream state of a host. A blank email of an account becomes no email. The migration 0055 replaces the unique keys of the names. The migrations 0056 and 0057 move the repository to the plan and add the profile parameters. See the sections below.
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

### All agents

- The service stops with the exit code 78 when its identity, its program or its restic is not safe. A restic that is missing or that differs from its record also stops the service. The service checks the identity again at each new connection. It does not restart after the exit code 78. `restichub-agent status` shows the cause. For a missing restic, run `install-service --restic-archive <ARCHIVE>`.

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
3. Run `restichub-agent register --force --url <URL> --token <TOKEN>` as Administrator. The command repairs all the directories first. It removes the old identity only after all the checks pass.
4. If a command printed `repaired the access control of ... under C:\ProgramData\ResticHub...`, restart the computer. A program that opened that directory before the repair keeps the access of its handle until the restart. With that handle it can put back its access entry. The service then refuses the identity and stops with the exit code 78.
5. After the restart, run `install-service` again. Its output must not show a line that starts with `repaired`. If it shows such a line, a program with administrator rights changes the rights of the directory. An example is a management tool or a sync tool. Find that program and stop it. Then do the steps 4 and 5 again.

A host that you registered first with 0.1.0-rc.2 or a later release on a clean computer does not need these steps. `install-service` alone is sufficient there.

### Linux agents

`install-service` writes the systemd unit again and removes a change that you made by hand. The new unit has a system call filter (`SystemCallFilter=@system-service`) and hides the other processes (`ProtectProc=invisible`). A system call outside the filter fails with the error `EPERM`. Thus a server script cannot use a debug tool such as `strace` or `perf`. See [Server scripts](/restichub/guides/scripts/).

The new unit also does not set `RestrictSUIDSGID`. A restore of a file with a setuid or setgid mode failed on a host with the older unit. Such a restore wrote the file without the setuid or setgid bit. Do the restore again after you run `install-service`. Then the restore keeps the mode.

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
4. Run `sudo restichub-agent install-service`. The command adds each root to `ReadWritePaths` and starts the service.
5. If you moved a repository, open it in "Repositories" and enter the new path in the field "Path".
6. Start "Test access" for each local repository. The result must be "The repository is ready.".

A repository root cannot be `/`, a system directory such as `/etc` or `/usr`, or a directory of the agent. A restore directory and a repository root cannot hold each other, except the restore directory `/`. `install-service` stops with a message that names the line.

With the restore directory `/`, an in-place restore below `/home` or `/root` now fails with `read-only file system`. A restore directory `/home` or `/root` does not change this, and `install-service` now refuses such a directory. Use a directory below it that root owns. Restore to another directory, for example `/var/lib/restichub-restore`, and copy the files to their place. See [Restore directories](/restichub/guides/restore/#restore-directories).
