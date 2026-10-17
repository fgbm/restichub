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

The command stops the service, copies the new program, starts the service and waits for the connection. Use the package `linux-arm64` on an ARM host.

## Upgrade an agent on Windows

1. Download `restichub-agent-<version>-windows-amd64.zip` from the release page.
2. Unpack the archive in an empty folder, for example in "Downloads".
3. Start PowerShell with "Run as administrator", and go to the folder.
4. Install the new agent:

   ```powershell
   .\restichub-agent.exe install-service
   ```

The command stops the service, copies the new program to `%ProgramFiles%\ResticHub\Agent` and starts the service. It also repairs the access control of `%ProgramData%\ResticHub` that an older agent left. If an older agent or a manual copy left `%ProgramFiles%\ResticHub` open, the command makes it again. If the command stops with a message, see [Agent commands and files](/restichub/reference/agent-commands/#upgrade-the-agent).

If a job runs, the command cannot stop the service. Wait for the job, then run the command again.

The command cannot move an open `%ProgramFiles%\ResticHub` while a process keeps a file or a directory in it open. Do not go to a folder in `%ProgramFiles%\ResticHub` in PowerShell. If the move fails, the service stays stopped. Close that process or restart the computer, then run the command again.

## Check the agents

1. Open "Hosts".
2. Read the second line of each row. A current agent shows only its version, for example `agent 0.1.0`.
3. If a host shows "Agent too old", upgrade the agent of that host.

## Go back to the old release

If the new control plane does not start, go back to the old release:

1. Stop the stack with `docker compose down`.
2. Check out the old release tag, and set the old image reference in `.env`.
3. Restore the database from the backup of the step "Before you start". See [Back up the control plane](/restichub/operations/back-up-the-control-plane/#restore-on-a-new-host).
4. Start the stack with `docker compose up -d`.

Upgrade the agents only after the new control plane works.

## Upgrade notes since 0.1.0-rc.4

- Three database migrations run at the first start. They add indexes and fields of the lists, and the password reset of a platform account.
- On Windows, `install-service` and `register` repair the access control of `%ProgramData%\ResticHub`. Read the output: it prints one line for each top-level directory, and one line for each removal and each warning. Add `--verbose` to print each repaired item. An open `%ProgramFiles%\ResticHub` is moved aside and made again, so run the command from the unpacked package.
- The Windows service restarts after each failure, after 5, 30 and then 60 seconds. It does not restart after the exit code 78, as on Linux.
- A Windows service with an identity that another account could change does not start. It writes the cause to the event log `ResticHub`, and `status` shows it. Before this release such a service connected, and each job failed with "restic did not start".

### Windows hosts that need a new registration

The releases 0.1.0-rc.2, 0.1.0-rc.3 and 0.1.0-rc.4 make each missing level of `%ProgramData%\ResticHub` and `%ProgramFiles%\ResticHub` with the access control for SYSTEM and Administrators only. They do not make a directory with the inherited access. An open access has two causes. An agent build before 0.1.0-rc.2 made `%ProgramData%\ResticHub` with the access of `%ProgramData%`. A person made a folder by hand, for example for a manual copy of the agent to `%ProgramFiles%\ResticHub\Agent`.

On such a host, do these steps one time:

1. Run `install-service` of the new package as Administrator.
2. If the command says `the identity of this host could be changed by another account`, create a token with "Re-register" on the host page.
3. Run `restichub-agent register --force --url <URL> --token <TOKEN>` as Administrator. The command repairs all the directories first. It removes the old identity only after all the checks pass.
4. If a command printed `repaired the access control of ... under C:\ProgramData\ResticHub...`, restart the computer. A program that opened that directory before the repair keeps the access of its handle until the restart. With that handle it can put back its access entry. The service then refuses the identity and stops with the exit code 78.
5. After the restart, run `install-service` again. Its output must not show a line that starts with `repaired`. If it shows such a line, a program with administrator rights changes the rights of the directory, for example a management tool or a sync tool. Find that program and stop it. Then do the steps 4 and 5 again.

A host that you registered first with 0.1.0-rc.2 or a later release on a clean computer does not need these steps. `install-service` alone is sufficient there.
- The Windows service writes its log to rotated files in `%ProgramData%\ResticHub\Agent-logs` and to the event log `ResticHub`. See [Agent logs](/restichub/reference/agent-logs/).
- A local repository is bound to its host. The control plane refuses a job of a local repository on another host with the reason `not_repository_host`. If several hosts used one shared network disk (NFS or SMB) as a local repository, move this repository to rest-server or SFTP. To find and correct the old plans, see [Repositories saved before this rule](/restichub/guides/add-a-repository/#repositories-saved-before-this-rule).

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

A repository root cannot be `/`, a system directory such as `/etc` or `/usr`, or a directory of the agent. A restore root and a repository root cannot hold each other, except the restore root `/`. `install-service` stops with a message that names the line.

With the restore root `/`, an in-place restore below `/home` or `/root` now fails with `read-only file system`. A restore root `/home` or `/root` does not change this, and `install-service` now refuses such a root. Use a directory below it that root owns. Restore to another root, for example `/var/lib/restichub-restore`, and copy the files to their place. See [Restore roots](/restichub/guides/restore/#restore-roots).
