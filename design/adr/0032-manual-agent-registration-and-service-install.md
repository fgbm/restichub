# ADR-0032: Manual agent registration and service install

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-ID-01, FR-ID-02, FR-IN-01, FR-IN-07, FR-IN-08, FR-AG-01, NFR-SEC-05

## Context

FR-IN-03, FR-IN-04 and FR-IN-05 install an agent over the network. They are 1.0 features. The MVP has no automatic and no remote agent installation.

The enrollment token is a secret. T8 and T9 in [threat-model.md](../threat-model.md) cover a stolen token and a secret in a process argument. AGENTS.md forbids a secret in a process argument. The shell history, the process list and `/proc/<pid>/cmdline` all show a command-line argument.

The GitLab runner registration is the reference model. An administrator makes a one-time token in the web interface. The host runs one command, and the agent keeps its identity.

## Decision

- The Admin creates a one-time enrollment token in the Web UI, on the "Add host" screen. FR-IN-01 holds the generated command.
- The screen shows the ready registration command with the control plane URL. The screen shows the token value one time, as a separate item. The token is not part of the command text.
- A person unpacks the agent package on the host and runs `restichub-agent register --url https://<control-plane>`.
- `register` reads the token from the standard input. The option `--token-file <path>` reads the token from a file for an unattended run. The command has no `--token` flag.
- `register` generates the agent key pair. It sends the certificate signing request and the token to the enrollment endpoint (#48, ADR-0026). It stores its identity in the protected configuration directory (FR-AG-04).
- The identity is the agent private key, the agent certificate, the control plane root certificate, the control plane address and the public job signing key.
- Then the person runs `restichub-agent install-service`. The command writes and enables the operating system service. The service is a Windows service as LocalSystem, a systemd unit on Linux and a launchd daemon on macOS (FR-AG-01).
- The MVP does not install an agent over the network. FR-IN-03, FR-IN-04 and FR-IN-05 stay 1.0.

## Consequences

- The token never appears in the shell history and never in a process argument.
- The operator copies the token once, like the GitLab runner model. The token is one-time and has a short TTL.
- Each host needs one manual step. The MVP has no fleet installation.
- The registration and the service install are two commands. A person can register a host before the identity serves the service.
- An unattended run needs `--token-file`. The file with the token is a short-lived local file.

## Threats

| Threat | Control in this record |
|---|---|
| T21: a token in the shell history or in a process argument | `register` reads the token from the standard input or a file. The command has no token flag. The token is one-time with a short TTL (FR-ID-01). |

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| A `--token <value>` flag, as the GitLab runner has | The token enters the shell history, the process list and the audit trail of the shell. AGENTS.md forbids a secret in a process argument. |
| One command that registers and installs the service | The register step takes the token and the service step needs administrator rights. Two commands keep the secret out of the service configuration. |
| An installation script with the token embedded | The token lands in a file on the host. This is the 1.0 remote installation path (FR-IN-03). |
