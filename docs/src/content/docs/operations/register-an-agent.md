---
title: Register an agent
description: Register a host with the control plane and install the agent service.
---

An agent registers by hand with a one-time enrollment token. A person on the host unpacks the agent package anywhere, for example in the Downloads folder, and runs one command. The command `register` does everything: it installs the agent, enrolls the host, starts the service and reports the connection.

## The registration command

The Web UI shows the full command on the "Add host" screen. The command uses the control plane URL and the token:

```sh
restichub-agent register --url https://<control-plane> --token <token>
```

- `--url` is the control plane URL. The scheme must be `https`.
- `--token` is the one-time enrollment token. This is the usual way to pass the token.
- `--token-file PATH` reads the token from a file. A script uses this option.
- `--token-stdin` reads the token from the standard input.
- `--ca-file PATH` verifies the control plane with the CA in the file. Without this option the agent uses the system root certificates.
- `--force` replaces an existing identity. Use it to register the host again.
- `--no-service` enrolls the host and installs no service. Run `install-service` later.
- `--trust-server-scripts` turns on the local trust mode before the token is used. The control plane can then run any command on this host, as root or LocalSystem. Without this option the mode is off. See [Scripts](/restichub/operations/scripts/).
- `--restic-archive PATH`, `--restic-sums PATH` and `--restic-sums-sig PATH` give the restic release files of an offline host (see "The restic binary"). Give all three or none.

Run the command as `root` on Linux, or in an elevated PowerShell on Windows ("Run as administrator"). You can run it from any folder. The agent checks the rights, creates its directories and proves that it can write there before it contacts the control plane. A failed check does not use the token.

Give the token with exactly one of `--token`, `--token-file` and `--token-stdin`. The command refuses to run when an agent identity already exists on the host, unless you pass `--force`.

The command installs restic when the host has none. This step runs after the checks of the rights and the directories, and before the request to the control plane. A host without internet access and without the offline files fails here, and the token stays unused.

When the enrollment succeeds, the command starts the service and waits up to 60 seconds for the connection to the control plane. Then it prints a summary:

```text
registered as backup-host-01 (agent id 0192f1e0-0000-7000-8000-000000000001)
organization:  0192f1e0-0000-7000-8000-000000000002
service:       installed and running
connection:    connected to https://<control-plane>
```

If a step fails, the summary names what failed and the next command to run. The exit code is not zero when the agent is not connected. The identity stays, so the next command is `install-service` or `status`, not a new registration.

After `register --no-service` the command prints that the service is not installed and names `install-service` as the next command.

After a successful registration, the host appears on the "Hosts" screen with the status "Waiting for connection". The status changes to "Active" when the agent opens its stream.

The token is single use. The control plane consumes the token at the first successful registration. A used token, an expired token and a revoked token give a clear error.

## The host clock

The host clock must be within 5 minutes of the control plane clock. The control plane issues a certificate that starts 5 minutes before the issue time. The agent accepts a certificate that starts up to 5 minutes after the host time. A larger difference makes the certificate unusable.

Before it sends the token, the agent reads the time from the answer of the control plane. If the difference is more than 5 minutes, the agent stops and shows the local time, the control plane time and the difference. The token stays unused. The agent also shows which certificate is not valid when the check of the issued certificate fails.

The stream to the control plane uses the verification of the TLS library, which has no tolerance. Keep the host clock correct for the whole life of the agent.

To check and fix the clock:

- Windows: run `w32tm /query /status` in an elevated PowerShell, then `w32tm /resync`. Or open Settings, Time & language, Date & time, and select "Sync now". Set the time zone too.
- Linux: run `timedatectl status` and look for `System clock synchronized: yes`. Run `timedatectl set-ntp true` to enable the time sync. With chrony, run `chronyc makestep`.

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

The token never reaches a file and never reaches a log.

## Check the connection

The agent connects to the control plane after the service starts. The agent keeps one stream open and answers the heartbeat of the control plane. The stream uses mTLS. The agent verifies the control plane with the installation root CA only, never with the system root certificates.

To check the connection:

1. Open the "Hosts" screen in the Web UI.
2. Find the host by its name.
3. Read the status. A host that enrolled but did not connect yet shows "Waiting for connection". The status is "Active" after the first connection.
4. Read the "Last seen" time. The control plane writes this time while the stream is open. A time within the last minute shows a live connection.
5. Read the column "Agent version". The mark "Update available" shows an agent package older than the control plane (see [Agent and control plane versions](/restichub/operations/agent-version-compatibility/)).

The control plane sends a heartbeat every 30 seconds and closes the stream after three heartbeats without an answer. The Overview counts a host as online when the control plane heard from it in the last 90 seconds. The alert "Host offline" waits for its own limit (see [Alerts](/restichub/operations/alerts/)).

The agent writes one line when the control plane greets it:

```text
the control plane greeted the agent
```

An agent with a revoked or an unknown certificate writes this line:

```text
the control plane refused the agent identity; check that the agent is enrolled and not revoked
```

The agent then waits 60 seconds and tries again. Register the host again after a revoke (see [Enroll a host again](/restichub/operations/enroll-a-host-again/)).

## Initialize a repository

An agent runs a repository job on demand. The control plane sends the job through the open stream. The job holds a `test_access` step and, for an init, an `init` step. The agent runs `restic cat config` for a test-access step and `restic init` for an init step.

The agent runs `init` only when `test_access` exits with restic code 10. Restic code 10 means that the repository does not exist. Any other result stops the job, so the agent never initializes an existing repository a second time. A wrong password exits with code 12 and the job reports `config_error`.

The repository password and the backend credentials travel inside the signed job only. The agent passes them to restic in the environment. They never reach an argument, a log, the job journal or an error message. The agent keeps the result of a finished job, so a job that the control plane delivers again does not run a second time.

To initialize a repository:

1. Open the "Repositories" screen in the Web UI.
2. Find the repository.
3. Start "Initialize". For a repository that exists, start "Test access" instead.
4. Read the job result. "The repository is ready." means that restic initialized the repository or that the access worked. "The repository password or the credentials are not correct." is the result `config_error`. "The repository does not exist yet. Initialize it." is the result `not_initialized`.
5. Read the repository state. The control plane sets `initialized` after a successful init or a successful test-access.

An init and a local-path repository run on the maintenance agent of the repository only.

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
| `trust-server-scripts --enable` or `--disable` | Turn the local trust mode for [server scripts](/restichub/operations/scripts/) on or off. Run it as `root` or Administrator. The new state applies to the next job. The command restarts an idle service, so that the control plane sees the new state. It does not restart a service with a job in progress. Use `--no-restart` to skip the restart. |
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

An older install kept restic in `/var/lib/restichub-agent/bin`. An older Windows install kept it in `%ProgramFiles%\ResticHub\Agent\bin` or in `%ProgramData%\ResticHub\Agent\bin`. `register` and `install-service` stop a running service, move restic into the program directory and point the service at the new path. They remove the old copies and the empty `bin` directory. The agent checks the moved file against the hash in the record. The identity stays, so the host does not register again. A Windows service moves restic by itself when it starts with the new agent. It keeps working without a new `register`. To migrate an old install, unpack the new agent package and run `restichub-agent install-service` as `root` or Administrator.

On Linux the command writes `/etc/systemd/system/restichub-agent.service` and enables the unit. On Windows the command writes a Windows service that runs as LocalSystem. The service starts after a reboot.

### Upgrade the agent

To upgrade an agent, you do not register the host again. The identity stays.

1. Download the new agent package on the host and unpack it in an empty folder.
2. Run `restichub-agent install-service` from the unpacked package. Use an elevated PowerShell on Windows and `sudo` on Linux.

The command stops the running service, copies the new executable into the program directory, points the service at it and starts it. It keeps the identity and the restic record. It waits for the connection, as `register` does.

On Windows the command also repairs the access control of `%ProgramData%\ResticHub`, when an older agent left a foreign access there. The command first sets the protected access control on the directory. Then it examines the items in the directory. The command prints one line for each change. It does not read the old restore root `%ProgramData%\ResticHub\Restore`. The service never repairs the access.

The agent does not use a file that another account could change before the repair:

- If another account could change `Agent-server-scripts`, the command removes the file. The trust mode for server scripts is then off. To turn it on again, run `restichub-agent trust-server-scripts --enable` as Administrator.
- If another account could change `Agent-restore-roots.txt`, the command renames the file to `Agent-restore-roots.txt.untrusted-<time>`. The extra restore roots are then off. Examine the file, and then rename it back to turn them on again.
- In a directory that another account could change, the command removes the files of an incomplete certificate rotation: `rotation.next` and the files that end with `.next`.
- If another account could read or change the identity, the command shows a warning. Register the host again (see [Enroll a host again](/restichub/operations/enroll-a-host-again/)).
- Another account could change the `Agent` directory or a file of the identity, for example `ca-chain.pem`, `signing-keys.json` or `identity.json`. Then `install-service` does not use the identity and does not start the service. That account could replace the trust anchors of the agent. Register the host again with `register --force`.

The command stops at an item that it cannot repair. Such an item has another owner, is a link or a hard link, or is not a file of the agent. The message names the item.

On Linux the command needs no repair: it sets the mode of `/etc/restichub-agent` itself, and an old install leaves no directory that the command refuses.

### Troubleshooting

The exit code is 0 on success and 1 on an error. The exit code is 78 when the service starts without a usable identity. The systemd unit does not restart the service after the code 78, because a restart cannot create an identity.

| Message or symptom | Cause | Action |
| --- | --- | --- |
| `--url is required for register` | The command has no `--url`. | Add the control plane URL. |
| `give the token with --token, --token-file or --token-stdin` or `give exactly one of ...` | The command has no token source, or it has more than one. | Use one source only. |
| `an agent identity already exists in ...; pass --force to register again` | The host holds an identity. | Create a token with "Re-register" on the host record, and run `register --force` (see [Enroll a host again](/restichub/operations/enroll-a-host-again/)). |
| `run this command as an administrator ... or as root` | You did not run the command as `root` or Administrator. | Run it with `sudo`, or in an elevated PowerShell. |
| The error names a clock difference | The clock of the host differs by more than 5 minutes. | Fix the clock (see "The host clock"). The token stays unused. |
| The error says that restic cannot be downloaded | The host cannot reach GitHub. | Use a proxy or the offline files (see "The restic binary"). The token stays unused. |
| The control plane refuses the token | The token is used, expired or cancelled. | Create a new token with "Add host". |
| `register first: restichub-agent register ...` | `install-service` found no valid identity. | Run `register` with a new token. |
| `service: installed, not running` | The service stopped at the start. | On Linux read `journalctl -u restichub-agent`. On Windows read the Application log. Fix the cause and run `install-service`. |
| `connection: not connected ... after the wait` | The agent cannot reach the control plane. | Check the clock of the host, the firewall to port 8443 and the URL. Run `status` to read the last error. |
| `cannot load the agent identity in ...; register the agent again with a new enrollment token` | The identity directory is damaged or empty. The service exits with code 78 and does not restart. | Create a token with "Re-register" and run `register --force` with it. |
| The host stays in "Waiting for connection" | The service does not run, or it cannot reach the control plane. | Run `restichub-agent status` and read "last error". Check the service, the clock and the firewall. |
| `the control plane does not accept the protocol version of this agent` | The agent and the control plane have different protocol versions. | Update the agent (see [Agent and control plane versions](/restichub/operations/agent-version-compatibility/)). |
| `... cannot protect the directory ...ResticHub ... grants the access mask ... to an account that is not SYSTEM or Administrators` | An older agent left `%ProgramData%\ResticHub` with the access of `%ProgramData%`. | Run `restichub-agent install-service` of the new package as Administrator. The command repairs the access and keeps the identity. Do not delete the directory, because it holds the identity. |
| `... holds ..., which is not a file of the agent`, `... is owned by an account that is not SYSTEM or Administrators, so the agent does not use it`, `... is a hard link ...` or `... is a reparse point ...` | The directory holds a file that the agent does not know, a file that another account owns, a hard link or a link. | Check the named file. Remove that file, not the directory. Run the command again. If the file is a part of the identity, for example `agent-key.pem`, register the host again after the removal. |
| `the identity of this host could be changed by another account (...)` | An older agent let another account change the `Agent` directory or a file of the identity. The repair stopped that access, but that account could replace the CA chain, the signing keys or the endpoint. `install-service` does not install the service. | Create a token with "Re-register" on the host page. Run `restichub-agent register --force --url <URL> --token <TOKEN>` (see [Enroll a host again](/restichub/operations/enroll-a-host-again/)). |
| `warning: another account could read or change ..., so the agent identity may be exposed` | An older agent let another account read or change the identity. The repair stopped that access, but the key can be already known. | Register the host again (see [Enroll a host again](/restichub/operations/enroll-a-host-again/)). |
| `... is owned by an account that is not SYSTEM or Administrators, so that account may have put files there` | Another account owns a directory of the agent. The agent does not repair it. | Check the files in the directory. Then run the two PowerShell lines from the message: `icacls <directory> /setowner *S-1-5-32-544` and `Set-Acl` with the SDDL `O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`. Run the command again. |
| The command refuses a directory or file | Another account owns the item or can write it. | Reset the owner and the access to SYSTEM and Administrators (Linux: `root`, mode 0755). Run the command again. |
| `cannot stop the service` on Windows | A job still runs. | Wait for the job, or stop the service by hand. Run the command again. |
| `the restic binary ... is missing` on Windows after an update | An older agent kept restic in the `bin` directory, and the service could not move it. The log of the service shows `cannot remove` or `cannot replace`. | Close the program that holds the old file, or clear its read-only attribute. Then run `restichub-agent install-service` as Administrator. The command moves restic to `%ProgramFiles%\ResticHub\Agent\restic.exe`. |
| `restic differs from its record after the move` | The old file changed before the move. | Run `restichub-agent restic update` as Administrator. |

## The systemd hardening trade-off

The unit keeps five capabilities: `CAP_DAC_READ_SEARCH`, `CAP_DAC_OVERRIDE`, `CAP_CHOWN`, `CAP_FOWNER` and `CAP_FSETID`. Restic runs as `root`, so these five are the only capabilities that the bounding set keeps. Without them restic cannot read a file of another account, or keep the owner and the mode of a restored file.

`ProtectSystem=strict` makes the whole file system read-only, except the identity directory through `ReadWritePaths` and the cache directory. A read of a backup source stays possible. Restic writes to a local repository, so add every local repository path to `ReadWritePaths`. The unit also lists the restore roots there (`/var/lib/restichub-restore` and each root from `/etc/restichub-agent-restore-roots.txt`). `install-service` creates the default root. Run it again after you add a root, because systemd reads the list at the start of the unit. `ProtectHome` stays off, because a backup source is often below `/home`.

The unit sets `CacheDirectory=restichub-agent` and `RESTIC_CACHE_DIR=/var/cache/restichub-agent`, so the restic cache stays in a directory that the unit can write.

## The restic binary

The agent package does not hold restic. The agent downloads restic itself. `register` and `install-service` download it when the host has none. `run` never downloads. The service makes no network request for restic.

The agent takes the latest release of restic from GitHub. It verifies the OpenPGP signature of the file `SHA256SUMS` with the release signing key of the restic project, which is part of the agent. Then it checks the hash of the archive. It installs only restic 0.18.1 or later, and it refuses a release of a major version above 0. A failed check installs nothing.

The agent stores the binary in a protected directory:

- Linux: `/opt/restichub-agent/bin/restic`.
- Windows: `%ProgramFiles%\ResticHub\Agent\restic.exe`, next to the agent executable. Only SYSTEM and Administrators can write there.

The protected configuration directory holds the file `restic.json` with the version and the SHA-256 of the installed binary. At each start, and before each restic process, the agent checks the binary against this record and refuses a different binary. The control plane cannot set the restic path. If restic is absent or does not match, the service stops with a message that names the next step: run `restichub-agent restic update`.

### Update restic

Run the command as `root` or in an elevated PowerShell:

```sh
restichub-agent restic update
```

The command downloads the latest release and verifies it. It runs the new binary with `version`. Only then does it replace the old binary with one rename. The old binary stays in the same directory as `restic-previous`. If a step fails, the old binary and the old record stay as they were. The service reads the record before each restic process, so the next job uses the new binary. If a service is installed, the command restarts it. Use `--no-restart` to restart it yourself later. On Windows the replacement fails while the service runs the binary. Then stop the service, run the command again and start the service.

The command refuses a release that is older than the installed one. Use `--allow-downgrade` to install it.

The latest release can fail the checks. Examples are a new major version, a changed signature format or a GitHub rate limit. Then the command takes the newest supported release of the last ten GitHub releases. If that also fails, it uses the release that this agent build pins. If no release installs, the message lists each failure. Then use the offline files.

### An offline host

A host without access to GitHub uses files that you copied from another computer. Download these three files from the restic release page of the version that you want:

- `restic_<version>_linux_amd64.bz2` (Linux) or `restic_<version>_windows_amd64.zip` (Windows). Use `arm64` for an ARM host.
- `SHA256SUMS`
- `SHA256SUMS.asc`

Do not rename the archive, because its name gives the version. Then run any of the commands `register`, `install-service` or `restic update` with the options:

```sh
restichub-agent restic update \
  --restic-archive ./restic_0.19.1_linux_amd64.bz2 \
  --restic-sums ./SHA256SUMS \
  --restic-sums-sig ./SHA256SUMS.asc
```

The agent verifies these files in the same way as a download.

### The proxy of the download

The download honours `HTTPS_PROXY` and `NO_PROXY` of the process that runs the command. The proxy must be an `http://` URL, for example `http://proxy.example:3128`. The agent opens a `CONNECT` tunnel through it, and the TLS session and all checks stay between the agent and GitHub. A proxy with a user name and a password uses `http://user:password@proxy.example:3128`. Write a special character of the user name or the password as a percent escape, for example `%40` for `@`. An IPv6 proxy address uses brackets, for example `http://[2001:db8::1]:3128`.

`sudo` removes these variables. Keep them with `sudo --preserve-env=HTTPS_PROXY,NO_PROXY restichub-agent restic update`.

GitHub limits the number of requests without a token. If GitHub answers with a rate limit, the agent says so. Try again later or use the offline files.

## Proxy settings

A repository behind a proxy uses the proxy of the host. The agent passes `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` from its own environment to the restic child process, and the lower-case forms on Linux and macOS. Set these variables in the environment of the agent service, never in a job.

The restic child holds no other variable of the agent, so a job cannot redirect the traffic of a repository.

## Related

- [Manual agent registration and service install](https://github.com/fgbm/restichub/blob/main/design/adr/0032-manual-agent-registration-and-service-install.md)
- [Agent downloads restic](https://github.com/fgbm/restichub/blob/main/design/adr/0038-agent-downloads-restic.md)
- [Enroll a host again and find duplicate hosts](/restichub/operations/enroll-a-host-again/)
- [Environment variables](/restichub/operations/environment-variables/)
- [Agent enrollment rate limit](enrollment-rate-limit.md)
- [Agent certificate rotation](certificate-rotation.md)
- [Agent and control plane versions](agent-version-compatibility.md)
