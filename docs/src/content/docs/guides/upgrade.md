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

The command stops the service, copies the new program to `%ProgramFiles%\ResticHub\Agent` and starts the service. It also repairs the access control of `%ProgramData%\ResticHub` that an older agent left. If the command stops with a message, see [Agent commands and files](/restichub/reference/agent-commands/#upgrade-the-agent).

If a job runs, the command cannot stop the service. Wait for the job, then run the command again.

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
- On Windows, `install-service` repairs the access control of `%ProgramData%\ResticHub`. Read its output: it prints one line for each change.
- The Windows service writes its log to rotated files in `%ProgramData%\ResticHub\Agent-logs` and to the event log `ResticHub`. See [Agent logs](/restichub/reference/agent-logs/).
