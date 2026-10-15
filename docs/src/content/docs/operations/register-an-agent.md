---
title: Register an agent
description: Register a host with the control plane and install the agent service.
---

An agent registers by hand with a one-time enrollment token (ADR-0032). A person on the host unpacks the agent package and runs one command.

## The registration command

The Web UI shows the full command on the "Add host" screen. The command uses the control plane URL and the token:

```sh
restichub-agent register --url https://<control-plane> --token <token>
```

- `--url` is the control plane URL. The scheme must be `https`.
- `--token` is the one-time enrollment token. ADR-0032 makes this flag the documented default.
- `--token-file PATH` reads the token from a file. A script uses this option.
- `--token-stdin` reads the token from the standard input.
- `--ca-file PATH` verifies the control plane with the CA in the file. Without this option the agent uses the system root certificates.
- `--force` replaces an existing identity. Use it to register the host again.
- `--restic-archive PATH`, `--restic-sums PATH` and `--restic-sums-sig PATH` give the restic release files of an offline host (see "The restic binary"). Give all three or none.

Run the command as `root` on Linux, or in an elevated PowerShell on Windows ("Run as administrator"). The agent checks the rights, creates its directories and proves that it can write there before it contacts the control plane. A failed check does not use the token.

The command installs restic when the host has none (ADR-0038). This step runs after the checks of the rights and the directories, and before the request to the control plane. A host without internet access and without the offline files fails here, and the token stays unused.

The token is single use. The control plane consumes the token at the first successful registration. A used token, an expired token and a revoked token give a clear error.

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
- `signing-key-updates.json`: the signed key updates that the agent accepted. The agent verifies a job after a key rotation with this chain (ADR-0030).
- `job-journal.json`: the accepted and completed jobs. A job that the control plane delivers again does not run a second time (ADR-0020).

The token never reaches a file and never reaches a log.

## Check the connection

The agent connects to the control plane after the service starts. The agent keeps one stream open and answers the heartbeat of the control plane (FR-ID-07). The stream uses mTLS. The agent verifies the control plane with the installation root CA only, never with the system root certificates.

To check the connection:

1. Open the "Hosts" screen in the Web UI.
2. Find the host by its name.
3. Read the status. The status is `active` when the agent is enrolled.
4. Read the "Last seen" time. The control plane writes this time while the stream is open. A time within the last minute shows a live connection.

The control plane marks an agent offline after a grace period of 300 seconds without a heartbeat (FR-ID-08). A short network failure therefore does not change the status.

The agent writes one line when the control plane greets it:

```text
the control plane greeted the agent
```

An agent with a revoked or an unknown certificate writes this line:

```text
the control plane refused the agent identity; check that the agent is enrolled and not revoked
```

The agent then waits 60 seconds and tries again. Register the host again after a revoke (see above).

## Initialize a repository

An agent runs a repository job on demand. The control plane sends the job through the open stream. The job holds a `test_access` step and, for an init, an `init` step. The agent runs `restic cat config` for a test-access step and `restic init` for an init step.

The agent runs `init` only when `test_access` exits with restic code 10. Restic code 10 means that the repository does not exist. Any other result stops the job, so the agent never initializes an existing repository a second time. A wrong password exits with code 12 and the job reports `config_error`.

The repository password and the backend credentials travel inside the signed job only. The agent passes them to restic in the environment. They never reach an argument, a log, the job journal or an error message. The agent keeps the result of a finished job, so a job that the control plane delivers again does not run a second time (ADR-0020).

To initialize a repository:

1. Open the "Repositories" screen in the Web UI.
2. Find the repository.
3. Start "Initialize". For a repository that exists, start "Test access" instead.
4. Read the job state. The state `succeeded` means that restic initialized the repository, or that the access succeeded. The state `config_error` means that the password or the credentials are wrong. The state `not_initialized` means that the repository does not exist.
5. Read the repository state. The control plane sets `initialized` after a successful init or a successful test-access.

An init and a local-path repository run on the maintenance agent of the repository only.

## Install the service

Put the agent where only `root` or an administrator can change it. The recommended path is `/opt/restichub-agent` on Linux and `%ProgramFiles%\ResticHub\Agent` on Windows. The command refuses a binary that another account can write, and a directory that SYSTEM and Administrators do not own alone (threat T20).

Run the command as `root` on Linux, or as an administrator on Windows:

```sh
restichub-agent install-service
```

The command installs restic first when the host has none (see "The restic binary"). On Linux the command writes `/etc/systemd/system/restichub-agent.service` and enables the unit. On Windows the command writes a Windows service that runs as LocalSystem. The service starts after a reboot. The command `uninstall-service` stops and removes the service. Both commands are idempotent.

## The systemd hardening trade-off

The unit keeps five capabilities: `CAP_DAC_READ_SEARCH`, `CAP_DAC_OVERRIDE`, `CAP_CHOWN`, `CAP_FOWNER` and `CAP_FSETID`. Restic runs as `root`, so these five are the only capabilities that the bounding set keeps. Without them restic cannot read a file of another account, or keep the owner and the mode of a restored file.

`ProtectSystem=strict` makes the whole file system read-only, except the identity directory through `ReadWritePaths` and the cache directory. A read of a backup source stays possible. Restic writes to a local repository, so add every local repository path to `ReadWritePaths`. The unit also lists the restore roots there (`/var/lib/restichub-restore` and each root from `/etc/restichub-agent-restore-roots.txt`). `install-service` creates the default root. Run it again after you add a root, because systemd reads the list at the start of the unit. `ProtectHome` stays off, because a backup source is often below `/home`.

The unit sets `CacheDirectory=restichub-agent` and `RESTIC_CACHE_DIR=/var/cache/restichub-agent`, so the restic cache stays in a directory that the unit can write.

## The restic binary

The agent package does not hold restic. The agent downloads restic itself (ADR-0038). `register` and `install-service` download it when the host has none. `run` never downloads. The service makes no network request for restic.

The agent takes the latest release of restic from GitHub. It verifies the OpenPGP signature of the file `SHA256SUMS` with the release signing key of the restic project, which is part of the agent. Then it checks the hash of the archive. It installs only restic 0.18.1 or later, and it refuses a release of a major version above 0. A failed check installs nothing.

The agent stores the binary in a protected directory:

- Linux: `/var/lib/restichub-agent/bin/restic`.
- Windows: `%ProgramData%\ResticHub\Agent\bin\restic.exe`. Only SYSTEM and Administrators have access.

The protected configuration directory holds the file `restic.json` with the version and the SHA-256 of the installed binary. At each start, and before each restic process, the agent checks the binary against this record and refuses a different binary (threat T20). The control plane cannot set the restic path. If restic is absent or does not match, the service stops with a message that names the next step: run `restichub-agent restic update`.

### Update restic

Run the command as `root` or in an elevated PowerShell:

```sh
restichub-agent restic update
```

The command downloads the latest release and verifies it. It runs the new binary with `version`. Only then does it replace the old binary with one rename. The old binary stays in the same directory as `restic-previous`. If a step fails, the old binary and the old record stay as they were. Restart the agent service after an update, because the service reads the record at its start.

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

The download honours `HTTPS_PROXY` and `NO_PROXY` of the process that runs the command. The proxy must be an `http://` URL, for example `http://proxy.example:3128`. The agent opens a `CONNECT` tunnel through it, and the TLS session and all checks stay between the agent and GitHub. A proxy with a user name and a password uses `http://user:password@proxy.example:3128`.

GitHub limits the number of requests without a token. If GitHub answers with a rate limit, the agent says so. Try again later or use the offline files.

## Proxy settings

A repository behind a proxy uses the proxy of the host. The agent passes `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` from its own environment to the restic child process, and the lower-case forms on Linux and macOS. Set these variables in the environment of the agent service, never in a job.

The restic child holds no other variable of the agent, so a job cannot redirect the traffic of a repository (FR-AG-06).

## Related

- [ADR-0032](https://github.com/fgbm/restichub/blob/main/design/adr/0032-manual-agent-registration-and-service-install.md)
- [ADR-0031](https://github.com/fgbm/restichub/blob/main/design/adr/0031-bundled-restic-in-agent-package.md)
- [ADR-0038](https://github.com/fgbm/restichub/blob/main/design/adr/0038-agent-downloads-restic.md)
- [Agent enrollment rate limit](enrollment-rate-limit.md)
