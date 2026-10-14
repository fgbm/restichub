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

## Install the service

Put the agent where only `root` or an administrator can change it. The recommended path is `/opt/restichub-agent` on Linux and `%ProgramFiles%\ResticHub\Agent` on Windows. The command refuses a binary that another account can write, and a directory that SYSTEM and Administrators do not own alone (threat T20).

Run the command as `root` on Linux, or as an administrator on Windows:

```sh
restichub-agent install-service
```

On Linux the command writes `/etc/systemd/system/restichub-agent.service` and enables the unit. On Windows the command writes a Windows service that runs as LocalSystem. The service starts after a reboot. The command `uninstall-service` stops and removes the service. Both commands are idempotent.

## The systemd hardening trade-off

The unit keeps five capabilities: `CAP_DAC_READ_SEARCH`, `CAP_DAC_OVERRIDE`, `CAP_CHOWN`, `CAP_FOWNER` and `CAP_FSETID`. Restic runs as `root`, so these five are the only capabilities that the bounding set keeps. Without them restic cannot read a file of another account, or keep the owner and the mode of a restored file.

`ProtectSystem=strict` makes the whole file system read-only, except the identity directory through `ReadWritePaths` and the cache directory. A read of a backup source stays possible. Restic writes to a local repository, so add every local repository path to `ReadWritePaths`. `ProtectHome` stays off, because a backup source is often below `/home`.

The unit sets `CacheDirectory=restichub-agent` and `RESTIC_CACHE_DIR=/var/cache/restichub-agent`, so the restic cache stays in a directory that the unit can write.

## The restic binary

The agent package holds `restic` next to `restichub-agent`. The build embeds the expected SHA-256 of the restic binary. At the start of `register` and `run` the agent checks the hash and refuses a different binary (ADR-0031). The control plane cannot set the restic path.

## Proxy settings

A repository behind a proxy uses the proxy of the host. The agent passes `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` from its own environment to the restic child process, and the lower-case forms on Linux and macOS. Set these variables in the environment of the agent service, never in a job.

The restic child holds no other variable of the agent, so a job cannot redirect the traffic of a repository (FR-AG-06).

## Related

- [ADR-0032](https://github.com/fgbm/restichub/blob/main/design/adr/0032-manual-agent-registration-and-service-install.md)
- [ADR-0031](https://github.com/fgbm/restichub/blob/main/design/adr/0031-bundled-restic-in-agent-package.md)
- [Agent enrollment rate limit](enrollment-rate-limit.md)
