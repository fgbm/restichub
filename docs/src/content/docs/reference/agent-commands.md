---
title: Agent commands and files
description: The commands of the agent, its files and directories, and the fixes for common errors.
---

This page lists the commands of the agent and the files that it keeps. To add a new host, see [Add a host](/restichub/getting-started/add-a-host/).

## Install the service

`register` installs the service already. Use `install-service` when you registered with `--no-service`, or when the service needs a repair. The command refuses a host without an identity and tells you to run `register` first. It is idempotent. It starts the service and waits for the connection like `register`.

Run the commands as `root` on Linux, or as an administrator on Windows:

```sh
restichub-agent install-service
restichub-agent status
restichub-agent uninstall-service
```

- `status` prints the agent ID and the organization ID. It prints the certificate expiry, the restic version and path, and the service state. It also prints the control plane URL, the connection state, the time of the last connection and the last error. It prints no secret. It exits with the code 1 when the agent is not connected.
- `uninstall-service` stops and removes the service. It keeps the data, so `install-service` can install the service again.

A `status` output looks like this:

```text
agent id:            0192f1e0-0000-7000-8000-000000000001
organization:        0192f1e0-0000-7000-8000-000000000002
certificate expires: 2026-12-01 10:00:00.0 +00:00:00
restic:              restic 0.19.1 (/opt/restichub-agent/bin/restic)
service:             installed, running, start type automatic
control plane:       https://<control-plane>:8443
connection:          connected
last connected:      2026-10-15 09:30:12.0 +00:00:00
```

All commands of the agent:

| Command | Function |
| --- | --- |
| `register` | Enroll the host, install the files, download restic, install and start the service, and wait for the connection. |
| `install-service` | Install and start the service for an existing identity. Download restic if it is absent. |
| `restic update` | Download the latest restic and replace the installed one. |
| `uninstall-service` | Stop and remove the service. |
| `status` | Print the state of the agent. |
| `trust-server-scripts --enable` or `--disable` | Turn the local trust mode for [server scripts](/restichub/guides/scripts/) on or off. Run it as `root` or Administrator. The new state applies to the next job. The command restarts an idle service, so that the control plane sees the new state. It does not restart a service with a job in progress. Use `--no-restart` to skip the restart. |
| `run` | Run the agent in the foreground. The service uses this command. You do not start it by hand. |
| `version`, `--version` or `-V` | Print the agent version and the protocol version, for example `restichub-agent 0.1.0+3b21922 (protocol 1.1)`. |
| `help`, `--help` or `-h` | Print the usage. |

The agent installs itself, so you do not copy files by hand. The command copies the running agent into the program directory. Only SYSTEM and Administrators (Linux: `root`) can write there. If the service runs from that directory, the command stops it, replaces the file and starts it again. The service starts from the program directory. The command refuses an executable that another account can write.

| Item | Linux | Windows |
| --- | --- | --- |
| Agent executable | `/opt/restichub-agent/bin/restichub-agent` | `%ProgramFiles%\ResticHub\Agent\restichub-agent.exe` |
| Restic | `/opt/restichub-agent/bin/restic` | `%ProgramFiles%\ResticHub\Agent\restic.exe` |
| Identity, status file, restic record, cache, journal | `/etc/restichub-agent`, `/var/lib/restichub-agent`, `/var/cache/restichub-agent` | `%ProgramData%\ResticHub\Agent` |
| Default restore root | `/var/lib/restichub-restore` | `%ProgramData%\ResticHub-Restore` |
| Service log | journald | `%ProgramData%\ResticHub\Agent-logs` and the `ResticHub` event log |

An older install kept restic in `/var/lib/restichub-agent/bin`. An older Windows install kept it in `%ProgramFiles%\ResticHub\Agent\bin` or in `%ProgramData%\ResticHub\Agent\bin`. `register` and `install-service` stop a running service, move restic into the program directory and point the service at the new path. They remove the old copies and the empty `bin` directory. The agent checks the moved file against the hash in the record. The identity stays, so the host does not register again. A Windows service moves restic by itself when it starts with the new agent. It keeps working without a new `register`. To migrate an old install, unpack the new agent package and run `restichub-agent install-service` as `root` or Administrator.

On Linux the command writes `/etc/systemd/system/restichub-agent.service` and enables the unit. On Windows the command writes a Windows service that runs as LocalSystem. The service starts after a reboot.

## The stored identity

The command writes the identity into the protected configuration directory:

- Linux: `/etc/restichub-agent/`, owned by the service user or `root`. The directory mode is `0700`.
- Windows: `%ProgramData%\ResticHub\Agent\`. An ACL gives full control to SYSTEM and to Administrators only.

The directory holds these files:

- `agent-key.pem`: the agent private key. The mode is `0600` on Linux.
- `agent-cert.pem`: the agent certificate. The certificate holds the agent ID and the tenant ID.
- `ca-chain.pem`: the issuing CA and the root CA.
- `identity.json`: the agent ID, the tenant ID, the control plane URL and the gRPC endpoint.
- `signing-keys.json`: the job signing key set from the enrollment.
- `signing-key-updates.json`: the signed key updates that the agent accepted. The agent verifies a job after a key rotation with this chain.
- `job-journal.json`: the accepted and completed jobs. A job that the control plane delivers again does not run a second time.

An empty lock file keeps two agent processes from changing the identity at the same time: `/etc/restichub-agent/identity.lock` on Linux and `%ProgramData%\ResticHub\Agent.lock` on Windows.

The token never reaches a file and never reaches a log.

## Upgrade the agent

To upgrade an agent, you do not register the host again. The identity stays.

1. Download the new agent package on the host and unpack it in an empty folder.
2. Run `restichub-agent install-service` from the unpacked package. Use an elevated PowerShell on Windows and `sudo` on Linux.

The command stops the running service, copies the new executable into the program directory, points the service at it and starts it. It keeps the identity and the restic record. It waits for the connection, as `register` does.

On Windows the command also repairs the access control of `%ProgramData%\ResticHub`, when an older agent left a foreign access there. The command first sets the protected access control on the directory. Then it examines the items in the directory. The command prints one line for each change. It does not read the old restore root `%ProgramData%\ResticHub\Restore`. The service never repairs the access.

The agent does not use a file that another account could change before the repair:

- If another account could change `Agent-server-scripts`, the command removes the file. The trust mode for server scripts is then off. To turn it on again, run `restichub-agent trust-server-scripts --enable` as Administrator.
- If another account could change `Agent-restore-roots.txt`, the command renames the file to `Agent-restore-roots.txt.untrusted-<time>`. The extra restore roots are then off. Examine the file, and then rename it back to turn them on again.
- In a directory that another account could change, the command removes the files of an incomplete certificate rotation: `rotation.next` and the files that end with `.next`.
- If another account could read or change the identity, the command shows a warning. Register the host again (see [Enroll a host again](/restichub/guides/enroll-a-host-again/)).
- Another account could change the `Agent` directory or a file of the identity, for example `ca-chain.pem`, `signing-keys.json` or `identity.json`. Then `install-service` does not use the identity and does not start the service. That account could replace the trust anchors of the agent. Register the host again with `register --force`.

The command stops at an item that it cannot repair. Such an item has another owner, is a link or a hard link, or is not a file of the agent. The message names the item.

On Linux the command needs no repair: it sets the mode of `/etc/restichub-agent` itself, and an old install leaves no directory that the command refuses.

## Troubleshooting

The exit code is 0 on success and 1 on an error. The exit code is 78 when the service starts without a usable identity. The systemd unit does not restart the service after the code 78, because a restart cannot create an identity.

| Message or symptom | Cause | Action |
| --- | --- | --- |
| `--url is required for register` | The command has no `--url`. | Add the control plane URL. |
| `give the token with --token, --token-file or --token-stdin` or `give exactly one of ...` | The command has no token source, or it has more than one. | Use one source only. |
| `an agent identity already exists in ...; pass --force to register again` | The host holds an identity. | Create a token with "Re-register" on the host record, and run `register --force` (see [Enroll a host again](/restichub/guides/enroll-a-host-again/)). |
| `run this command as an administrator ... or as root` | You did not run the command as `root` or Administrator. | Run it with `sudo`, or in an elevated PowerShell. |
| The error names a clock difference | The clock of the host differs by more than 5 minutes. | Fix the clock (see [The host clock](/restichub/getting-started/add-a-host/#the-host-clock)). The token stays unused. |
| The error says that restic cannot be downloaded | The host cannot reach GitHub. | Use a proxy or the offline files (see [The restic binary](/restichub/reference/agent-packages/#the-restic-binary)). The token stays unused. |
| The control plane refuses the token | The token is used, expired or cancelled. | Create a new token with "Add host". |
| `register first: restichub-agent register ...` | `install-service` found no valid identity. | Run `register` with a new token. |
| `service: installed, not running` | The service stopped at the start. | On Linux read `journalctl -u restichub-agent`. On Windows read the `ResticHub` event log and the files in `%ProgramData%\ResticHub\Agent-logs` (see [Agent logs](/restichub/reference/agent-logs/)). Fix the cause and run `install-service`. |
| `connection: not connected ... after the wait` | The agent cannot reach the control plane. | Check the clock of the host, the firewall to port 8443 and the URL. Run `status` to read the last error. |
| `cannot load the agent identity in ...; register the agent again with a new enrollment token` | The identity directory is damaged or empty. The service exits with code 78 and does not restart. | Create a token with "Re-register" and run `register --force` with it. |
| The host stays in "Waiting for connection" | The service does not run, or it cannot reach the control plane. | Run `restichub-agent status` and read "last error". Check the service, the clock and the firewall. |
| `the control plane does not accept the protocol version of this agent` | The agent and the control plane have different protocol versions. | Update the agent (see [Agent and control plane versions](/restichub/reference/agent-version-compatibility/)). |
| `... cannot protect the directory ...ResticHub ... grants the access mask ... to an account that is not SYSTEM or Administrators` | An older agent left `%ProgramData%\ResticHub` with the access of `%ProgramData%`. | Run `restichub-agent install-service` of the new package as Administrator. The command repairs the access and keeps the identity. Do not delete the directory, because it holds the identity. |
| `... holds ..., which is not a file of the agent`, `... is owned by an account that is not SYSTEM or Administrators, so the agent does not use it`, `... is a hard link ...` or `... is a reparse point ...` | The directory holds a file that the agent does not know, a file that another account owns, a hard link or a link. | Check the named file. Remove that file, not the directory. Run the command again. If the file is a part of the identity, for example `agent-key.pem`, register the host again after the removal. |
| `the identity of this host could be changed by another account (...)` | An older agent let another account change the `Agent` directory or a file of the identity. That account could replace the CA chain, the signing keys or the endpoint. The repair does not change the access of these entries, so each `install-service` refuses them again. `install-service` does not install the service and stops a running service. The service stays installed. | Create a token with "Re-register" on the host page. Run `restichub-agent register --force --url <URL> --token <TOKEN>` (see [Enroll a host again](/restichub/guides/enroll-a-host-again/)). This command removes the old identity files and writes a new identity. |
| `warning: another account could read or change ..., so the agent identity may be exposed` | An older agent let another account read or change the identity. The repair stopped that access, but the key can be already known. | Register the host again (see [Enroll a host again](/restichub/guides/enroll-a-host-again/)). |
| `... is owned by an account that is not SYSTEM or Administrators, so that account may have put files there` | Another account owns a directory of the agent. The agent does not repair it. | Check the files in the directory. Then run the two PowerShell lines from the message: `icacls <directory> /setowner *S-1-5-32-544` and `Set-Acl` with the SDDL `O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`. Run the command again. |
| The command refuses a directory or file | Another account owns the item or can write it. | Reset the owner and the access to SYSTEM and Administrators (Linux: `root`, mode 0755). Run the command again. |
| `cannot stop the service` on Windows | A job still runs. | Wait for the job, or stop the service by hand. Run the command again. |
| `the restic binary ... is missing` on Windows after an update | An older agent kept restic in the `bin` directory, and the service could not move it. The log of the service shows `cannot remove` or `cannot replace`. | Close the program that holds the old file, or clear its read-only attribute. Then run `restichub-agent install-service` as Administrator. The command moves restic to `%ProgramFiles%\ResticHub\Agent\restic.exe`. |
| A job on a local repository fails with `read-only file system`, or with the reason `repository_path_denied` or `repository_read_only` | The systemd unit makes the file system read-only. The repository path is not below a repository root, or the installed unit does not list the root yet. | Add the root to `/etc/restichub-agent-repository-roots.txt` and run `sudo restichub-agent install-service` (see [A local repository on Linux](/restichub/guides/add-a-repository/#a-local-repository-on-linux)). |
| `restic differs from its record after the move` | The old file changed before the move. | Run `restichub-agent restic update` as Administrator. |

## The systemd hardening trade-off

The unit keeps five capabilities: `CAP_DAC_READ_SEARCH`, `CAP_DAC_OVERRIDE`, `CAP_CHOWN`, `CAP_FOWNER` and `CAP_FSETID`. Restic runs as `root`, so these five are the only capabilities that the bounding set keeps. Without them restic cannot read a file of another account, or keep the owner and the mode of a restored file.

`ProtectSystem=strict` and `ProtectHome=read-only` make the whole file system read-only, except the identity directory through `ReadWritePaths` and the cache directory. A read of a backup source stays possible. Restic writes to a local repository. The unit lists each repository root of `/etc/restichub-agent-repository-roots.txt` in `ReadWritePaths` (see [A local repository on Linux](/restichub/guides/add-a-repository/#a-local-repository-on-linux)). Do not edit the unit: `install-service` writes it again. The unit also lists the restore roots there (`/var/lib/restichub-restore` and each root from `/etc/restichub-agent-restore-roots.txt`). `install-service` creates the default root. Run it again after you add a root, because systemd reads the list at the start of the unit. A backup source below `/home` stays readable. `ProtectSystem=strict` alone does not cover `/home`, `/root` and `/run/user`, so the unit sets `ProtectHome=read-only` too.

The unit sets `CacheDirectory=restichub-agent` and `RESTIC_CACHE_DIR=/var/cache/restichub-agent`, so the restic cache stays in a directory that the unit can write.

## Proxy settings

A repository behind a proxy uses the proxy of the host. The agent passes `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` from its own environment to the restic child process, and the lower-case forms on Linux and macOS. Set these variables in the environment of the agent service, never in a job.

The restic child holds no other variable of the agent, so a job cannot redirect the traffic of a repository.
