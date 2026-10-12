# ADR-0022: SFTP is deferred to 1.0

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-RP-01, FR-AG-04, FR-AG-06

## Context

FR-RP-01 lists the repository backends. The current text gives SFTP priority M. The agent must give an SFTP private key to restic without a file on the host (FR-AG-04). The agent must pass secrets only through environment variables or a pipe (FR-AG-06).

Spike P6 compared five options. See [the spike report](../spikes/p6-sftp-key-delivery.md). One option gives the agent an SFTP key without a file on Linux and on Windows. This option is the restic `rclone:` backend with the rclone `key_pem` option in environment variables. rclone uses its own SSH client, so the SSH program is not necessary. The host key check uses the rclone `host_keys` option with no file.

Two facts limit this option. rclone adds about 86 MB to the agent package (the Linux amd64 binary of rclone v1.75.2 has 85,659,810 bytes). The test environment had no Windows host, so the Windows path is not verified.

## Decision

- We do not put SFTP in the MVP. The MVP backends are S3-compatible, rest-server and a local path.
- At 1.0 the project decides how to deliver rclone: in the agent package or downloaded on demand. The project also verifies the Windows path on a real host.
- The P6 rules become the basis of the SFTP implementation. These rules are fixed environment variable names, `SHELL_TYPE=none`, a mandatory host key for each job and strict PEM value rules.

## Consequences

- The MVP agent package stays small. The MVP has no third-party program that runs as root or LocalSystem.
- FR-RP-01 moves SFTP to priority 1.0. At 1.0, ADR-0021 and FR-AG-04 need the P6 value rules.
- A user who needs SFTP in the MVP has no supported path. The user can configure an rclone remote at 1.0 only.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| rclone in the MVP agent package | The package grows by about 86 MB. The Windows path is not verified. |
| Linux-only SFTP with ssh-agent in the MVP | The Windows agent has no working ssh-agent path (P6, option a). The MVP needs one code path for both systems. |
