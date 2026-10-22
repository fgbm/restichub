---
title: Agent commands and files
description: The commands of the agent, its files and directories, and the fixes for common errors.
---

This page lists the commands of the agent and the files that it keeps. To add a new host, see [Add a host](/restichub/getting-started/add-a-host/).

## Install the service

`register` installs the service already. Use `install-service` when you registered with `--no-service`, or when the service needs a repair. The command refuses a host without an identity and tells you to run `register` first. It is idempotent. It starts the service and waits for the connection like `register`.

Run the commands as `root` on Linux, or as an administrator on Windows. The examples show the full path of the installed agent, because the installer adds no link on `PATH`. On Windows the agent is `%ProgramFiles%\ResticHub\Agent\restichub-agent.exe`. This page names a command by its own name, for example `status`.

```sh
sudo /opt/restichub-agent/bin/restichub-agent install-service
sudo /opt/restichub-agent/bin/restichub-agent status
sudo /opt/restichub-agent/bin/restichub-agent uninstall-service
```

- `install-service`, `register`, `restic update` and `uninstall-service` refuse to run while a job runs in the service. The message `cannot stop the service` names the cause and the next step.
- `status` prints the agent ID and the organization ID. It prints the certificate expiry, the restic version and path, and the service state. It also prints the control plane URL, the connection state, the time of the last connection and the last error. It prints no secret. It exits with the code 1 when the agent is not connected.
- `uninstall-service` stops and removes the service. It keeps the data, so `install-service` can install the service again. It refuses while a job runs, as `install-service` does.

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
| Default restore directory | `/var/lib/restichub-restore` | `%ProgramData%\ResticHub-Restore` |
| Service log | journald | `%ProgramData%\ResticHub\Agent-logs` and the `ResticHub` event log |

An older install kept restic in `/var/lib/restichub-agent/bin`. An older Windows install kept it in `%ProgramFiles%\ResticHub\Agent\bin` or in `%ProgramData%\ResticHub\Agent\bin`. `register` and `install-service` stop a running service, move restic into the program directory and point the service at the new path. They remove the old copies and the empty `bin` directory. The agent checks the moved file against the hash in the record. The identity stays, so the host does not register again. A Windows service moves restic by itself when it starts with the new agent. It keeps working without a new `register`. To migrate an old install, unpack the new agent package and run `install-service` as `root` or Administrator.

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
2. Run `install-service` from the unpacked package. Use an elevated PowerShell on Windows and `sudo` on Linux.

The command stops the running service, copies the new executable into the program directory, points the service at it and starts it. It keeps the identity and the restic record. It waits for the connection, as `register` does.

On Windows the command also repairs the access control of `%ProgramData%\ResticHub` and `%ProgramFiles%\ResticHub`, when an older agent or a manual copy left a foreign access there. The command first sets the protected access control on the directory. Then it examines the items in the directory. For each top-level directory, the command prints one line with the number of repaired items, for example `repaired the access control of 812 entries under C:\ProgramData\ResticHub\Agent`. Each removal and each warning gets its own line. Add `--verbose` to `register` or `install-service` to print one line for each repaired item. The command does not read the old restore directory `%ProgramData%\ResticHub\Restore`. The service repairs only the entry that File Explorer adds for an administrator (see [Agent folders in File Explorer](#agent-folders-in-file-explorer)).

The repaired access control is the same as on a new install: full control for SYSTEM and Administrators, and the owner Administrators. Other users get no access, also no read access, because the service runs as LocalSystem and each command of the agent runs as Administrator. A folder that a person made by hand under `%ProgramFiles%` keeps the inherited access. On a client edition of Windows that access gives full control to the account of the administrator, also in a session without elevation. Then a program of that session can replace the agent or restic, or keep the folder open and add a file after the access changes. Thus the command does not repair `%ProgramFiles%\ResticHub` in place if it can move it. If another account could change or owns a level or an item of it, the command stops the service. The command removes the restic record before it changes the folder. Then it moves the open folder to `<name>.untrusted-<time>` and removes it. Next, it makes the folder again for SYSTEM and Administrators only and copies the running agent into it. It downloads and verifies restic again. If the restic record exists but restic does not, the command installs restic again. Run the command from the unpacked package. The current directory must not be in that folder. The command cannot move the folder that holds the running agent or the current directory. In these two cases it stops before it stops the service. If another process keeps a file in the folder open, the folder cannot move. Then the command repairs it in place. Each level from the open level to `Agent`, and each item in `Agent`, gets the access of SYSTEM and Administrators only, with the owner Administrators. The command removes the agent and restic and writes them again. It also removes each file that the agent does not know, as the move does. A program that runs is renamed to `<name>.untrusted-<time>`, and Windows removes it at the next restart. The output names the process that keeps the folder open and shows a warning. That process can add a file until the restart. Thus the command sets the manual start of the service and keeps a record of the repair in the registry key `HKLM\SOFTWARE\ResticHub\Agent`. While this record exists, each `install-service` and `register` checks the folder before the service starts. They refuse a file that the agent does not know, a link or an open access. Close the process, restart the computer and run `install-service` again. That run must not print a line that starts with `repaired` or `moved`. It sets the automatic start again and removes the record. `uninstall-service` also removes the record.

The agent does not use a file that another account could change before the repair:

- If another account could change `Agent-server-scripts`, the command removes the file. The trust mode for server scripts is then off. To turn it on again, run `trust-server-scripts --enable` as Administrator.
- If another account could change `Agent-restore-roots.txt`, the command renames the file to `Agent-restore-roots.txt.untrusted-<time>`. The extra restore directories are then off. Examine the file, and then rename it back to turn them on again.
- In a directory that another account could change, the command removes the files of an incomplete certificate rotation: `rotation.next` and the files that end with `.next`.
- If another account could read or change the identity, the command shows a warning. Register the host again (see [Enroll a host again](/restichub/guides/agent-certificates/)).
- Another account could change the `Agent` directory or a file of the identity, for example `ca-chain.pem`, `signing-keys.json` or `identity.json`. Then `install-service` does not use the identity and does not start the service. That account could replace the trust anchors of the agent. Register the host again with `register --force`.

The command stops at an item that it cannot repair. Such an item has another owner, is a link or a hard link, or is not a file of the agent. The message names the item.

`register --force` does all the checks and repairs before it removes an old identity: the program directory, the data directory and the state of the service. If a check fails, the old identity files stay in place.

The service does not repair, except the entry that File Explorer adds for an administrator. The service does not start if another account could change an item that the identity load uses. These items are the identity directory, its parent, the identity files, the backup `Agent.old` and the staged files of a certificate rotation. The key updates, the restic record and the job journal are also such items. The service does the same check each time it loads the identity again at a reconnect. It also does not start if another account could change the agent program, a directory above it, or restic. It writes the cause and the fix to the event log `ResticHub` (Linux: journald) and stops with the exit code 78. `status` shows the same cause in the line `identity`.

On Linux the command needs no repair: it sets the mode of `/etc/restichub-agent` itself, and an old install leaves no directory that the command refuses.

## Agent folders in File Explorer

On Windows, File Explorer can change the access control of an agent folder. The result depends on the folder and on the account:

- `%ProgramFiles%\ResticHub`: the Users group can read the files and run them. File Explorer opens the folder without a prompt and adds no access.
- `%ProgramData%\ResticHub` and its `Agent` folder: only SYSTEM and Administrators have access. File Explorer shows a prompt with "Continue". After the click, File Explorer adds an entry with full control for the signed-in account.

| Who opens the folder | What happens | What to do |
| --- | --- | --- |
| A local administrator clicks "Continue" | The service removes the entry at its next start, after a restart of the service or of the computer. A running service finds the entry at the next break of the connection to the control plane. It stops, Windows starts it again, and it removes the entry. `install-service` also removes the entry. The service writes a warning to the `ResticHub` event log. For the `Agent` folder, the warning tells that the agent identity was open to that account. | Nothing. You do not register the host again. If the warning names the `Agent` folder and you do not trust that account, register the host again (see [Enroll a host again](/restichub/guides/agent-certificates/)). |
| A user who is not an administrator, without an administrator password | File Explorer gives no access. | Nothing. |
| A user who is not an administrator, after an administrator typed the password at the prompt | File Explorer adds the entry for the signed-in user, not for the administrator. At each start the service stops with the exit code 78. | As an administrator, remove the entry: `icacls "<path>" /remove:g <account> /T`. Then run `install-service` in an elevated PowerShell. If the message names the agent identity, create a token with "Re-register" on the host page and run `register --force` with it. |

The folder can hold a file that the agent refuses. Then the service keeps the entry of an administrator too and stops with the exit code 78. The message names the file. See [Troubleshooting](#troubleshooting).

To read the warnings, use Event Viewer or an elevated PowerShell (see [Agent logs](/restichub/reference/agent-logs/)).

To look into the agent folders, do not use File Explorer. Use an elevated PowerShell, for example `Get-ChildItem "<path>"` and `icacls "<path>"`.

## Troubleshooting

The exit code is 0 on success and 1 on an error. The exit code is 78 when the service starts without a usable identity or with an identity that another account could change. It is also 78 when restic is missing or differs from its record, or when another account could change the agent program or restic. The service manager does not restart the service after the code 78, because only an administrator can fix the cause. After another failure the service manager restarts the service: systemd after 5 seconds, Windows after 5, 30 and then 60 seconds for each next failure. On Windows the count resets after one day without a failure.

| Message or symptom | Cause | Action |
| --- | --- | --- |
| `--url is required for register` | The command has no `--url`. | Add the control plane URL. |
| `give the token with --token, --token-file or --token-stdin` or `give exactly one of ...` | The command has no token source, or it has more than one. | Use one source only. |
| `an agent identity already exists in ...; pass --force to register again` | The host holds an identity. | Create a token with "Re-register" on the host record, and run `register --force` (see [Enroll a host again](/restichub/guides/agent-certificates/)). |
| `run this command as an administrator ... or as root` | You did not run the command as `root` or Administrator. | Run it with `sudo`, or in an elevated PowerShell. |
| The error names a clock difference | The clock of the host differs by more than 5 minutes. | Fix the clock (see [The host clock](/restichub/getting-started/add-a-host/#the-host-clock)). The token stays unused. |
| The error says that restic cannot be downloaded | The host cannot reach GitHub. | Use a proxy or the offline files (see [The restic binary](/restichub/reference/agent-packages/#the-restic-binary)). The token stays unused. |
| The control plane refuses the token | The token is used, expired or cancelled. | Create a new token with "Add host". |
| `register first: restichub-agent register ...` | `install-service` found no valid identity. | Run `register` with a new token. |
| `service: installed, not running` | The service stopped at the start. | On Linux read `journalctl -u restichub-agent`. On Windows read the `ResticHub` event log and the files in `%ProgramData%\ResticHub\Agent-logs` (see [Agent logs](/restichub/reference/agent-logs/)). Fix the cause and run `install-service`. |
| `connection: not connected ... after the wait` | The agent cannot reach the control plane. | Check the clock of the host, the firewall to port 8443 and the URL. Run `status` to read the last error. |
| `cannot load the agent identity in ...; register the agent again with a new enrollment token` | The identity directory is damaged or empty. The service exits with code 78 and does not restart. | Create a token with "Re-register" and run `register --force` with it. |
| `another account could change the agent program or restic: ...` | The agent program, a directory above it, or restic is writable by an account other than root or Administrators, for example after a manual copy. The service exits with code 78 and does not restart. | Run `install-service` of the release package as root or Administrator. It moves the agent into the protected program directory. |
| `the agent does not use the identity in ...: ... grants a write access to an account that is not SYSTEM or Administrators` | The service found an identity that another account could change. It did not connect. | Run `install-service` as Administrator. If it says that the identity could be changed by another account, register the host again with `register --force`. |
| The job page shows "restic did not start. Agent message: ..." | The agent could not start restic. The agent message is the cause, for example `cannot protect the directory ...`. | Fix the named cause. For a directory with a foreign access, run `install-service` as Administrator. |
| The host stays in "Waiting for connection" | The service does not run, or it cannot reach the control plane. | Run `status` and read "last error". Check the service, the clock and the firewall. |
| `the control plane does not accept the protocol version of this agent` | The agent and the control plane have different protocol versions. | Update the agent (see [Agent and control plane versions](/restichub/reference/agent-version-compatibility/)). |
| `... cannot protect the directory ...ResticHub ... grants the access mask ... to an account that is not SYSTEM or Administrators` | An older agent left `%ProgramData%\ResticHub` with the access of `%ProgramData%`. | Run `install-service` of the new package as Administrator. The command repairs the access and keeps the identity. Do not delete the directory, because it holds the identity. |
| `... holds ..., which is not a file of the agent`, `... is owned by an account that is not SYSTEM or Administrators, so the agent does not use it`, `... is a hard link ...` or `... is a reparse point ...` | The directory holds a file that the agent does not know, a file that another account owns, a hard link or a link. | Check the named file. Remove that file, not the directory. Run the command again. If the file is a part of the identity, for example `agent-key.pem`, register the host again after the removal. |
| `the identity of this host could be changed by another account (...)` | An older agent let another account change the `Agent` directory or a file of the identity. That account could replace the CA chain, the signing keys or the endpoint. The repair does not change the access of these entries, so each `install-service` refuses them again. `install-service` does not install the service and stops a running service. The service stays installed. | Create a token with "Re-register" on the host page. Run `register --force --url <URL> --token <TOKEN>` (see [Enroll a host again](/restichub/guides/agent-certificates/)). This command removes the old identity files and writes a new identity. |
| `warning: another account could read or change ..., so the agent identity may be exposed` | An older agent let another account read or change the identity. The repair stopped that access, but the key can be already known. | Register the host again (see [Enroll a host again](/restichub/guides/agent-certificates/)). |
| `... is owned by an account that is not SYSTEM or Administrators, so that account may have put files there` | Another account owns a directory of the agent. The agent does not repair it. | Check the files in the directory. Then run the two PowerShell lines from the message: `icacls <directory> /setowner *S-1-5-32-544` and `Set-Acl` with the SDDL `O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`. Run the command again. |
| The command refuses a directory or file | Another account owns the item or can write it. | Reset the owner and the access to SYSTEM and Administrators (Linux: `root`, mode 0755). Run the command again. |
| `cannot stop the service` | A job runs in the service. `install-service`, `register`, `restic update` and `uninstall-service` refuse, so the job is not cut short. | Wait for the job to end. Run the command again. A job of a crashed agent does not refuse the command. |
| `the restic binary ... is missing` on Windows after an update | An older agent kept restic in the `bin` directory, and the service could not move it. The log of the service shows `cannot remove` or `cannot replace`. | Close the program that holds the old file, or clear its read-only attribute. Then run `install-service` as Administrator. The command moves restic to `%ProgramFiles%\ResticHub\Agent\restic.exe`. |
| A job on a local repository fails with `read-only file system`, or with the reason `repository_path_denied` or `repository_read_only` | The systemd unit makes the file system read-only. The repository path is not below a repository root, or the installed unit does not list the root yet. | Add the root to `/etc/restichub-agent-repository-roots.txt` and run `sudo /opt/restichub-agent/bin/restichub-agent install-service` (see [A local repository on Linux](/restichub/getting-started/add-a-repository/#a-local-repository-on-linux)). |
| `restic differs from its record after the move` | The old file changed before the move. | Run `restic update` as Administrator. |

## The systemd hardening trade-off

The unit keeps five capabilities: `CAP_DAC_READ_SEARCH`, `CAP_DAC_OVERRIDE`, `CAP_CHOWN`, `CAP_FOWNER` and `CAP_FSETID`. Restic runs as `root`, so these five are the only capabilities that the bounding set keeps. Without them restic cannot read a file of another account, or keep the owner and the mode of a restored file.

`ProtectSystem=strict` and `ProtectHome=read-only` make the whole file system read-only, except the identity directory through `ReadWritePaths` and the cache directory. A read of a backup source stays possible. Restic writes to a local repository. The unit lists each repository root of `/etc/restichub-agent-repository-roots.txt` in `ReadWritePaths` (see [A local repository on Linux](/restichub/getting-started/add-a-repository/#a-local-repository-on-linux)). Do not edit the unit: `install-service` writes it again. The unit also lists the restore directories there (`/var/lib/restichub-restore` and each directory from `/etc/restichub-agent-restore-roots.txt`). `install-service` creates the default directory. Run it again after you add a directory, because systemd reads the list at the start of the unit. A backup source below `/home` stays readable. `ProtectSystem=strict` alone does not cover `/home`, `/root` and `/run/user`, so the unit sets `ProtectHome=read-only` too.

The unit does not set `RestrictSUIDSGID`. With that option systemd makes `chmod` refuse a setuid or setgid mode, so a restore of such a file, for example `/usr/bin/sudo`, fails. `NoNewPrivileges=true` and the capability bounding set stay, but they act only on the processes of the unit. A setuid program that a restore writes runs outside the unit, with the rights of its owner. Keep each restore directory at mode 0700, or mount it with `nosuid`.

The unit sets `CacheDirectory=restichub-agent` and `RESTIC_CACHE_DIR=/var/cache/restichub-agent`, so the restic cache stays in a directory that the unit can write.

## Proxy settings

A repository behind a proxy uses the proxy of the host. The agent passes `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` from its own environment to the restic child process, and the lower-case forms on Linux and macOS. Set these variables in the environment of the agent service, never in a job.

The restic child holds no other variable of the agent, so a job cannot redirect the traffic of a repository.
