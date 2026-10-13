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
- `signing-key-update.bin`: the signed job signing key set.

The token never reaches a file and never reaches a log.

## Install the service

Run the command as `root` on Linux, or as an administrator on Windows:

```sh
restichub-agent install-service
```

On Linux the command writes `/etc/systemd/system/restichub-agent.service` and enables the unit. On Windows the command writes a Windows service that runs as LocalSystem. The service starts after a reboot. The command `uninstall-service` stops and removes the service. Both commands are idempotent.

## The systemd hardening trade-off

The systemd unit hardens the service: `NoNewPrivileges=true`, `ProtectSystem=strict`, `PrivateTmp=true` and an empty capability set.

`ProtectSystem=strict` makes the whole file system read-only. The unit keeps the identity directory writable through `ReadWritePaths`. Restic reads the backup sources, and a read-only view permits a read. Restic writes to a local repository. Add every local repository path to `ReadWritePaths`. Without this change a backup to a local repository fails.

## The restic binary

The agent package holds `restic` next to `restichub-agent`. The build embeds the expected SHA-256 of the restic binary. At the start of `register` and `run` the agent checks the hash and refuses a different binary (ADR-0031). The control plane cannot set the restic path.

## Related

- [ADR-0032](https://github.com/fgbm/restichub/blob/main/design/adr/0032-manual-agent-registration-and-service-install.md)
- [ADR-0031](https://github.com/fgbm/restichub/blob/main/design/adr/0031-bundled-restic-in-agent-package.md)
- [Agent enrollment rate limit](enrollment-rate-limit.md)
