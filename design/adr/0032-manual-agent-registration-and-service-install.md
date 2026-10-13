# ADR-0032: Manual agent registration and service install

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-ID-01, FR-ID-02, FR-IN-01, FR-IN-07, FR-IN-08, FR-AG-01, NFR-SEC-05

## Context

FR-IN-03, FR-IN-04 and FR-IN-05 install an agent over the network. They are 1.0 features. The MVP has no automatic and no remote agent installation.

An enrollment token is a secret with a short life. T8 and T9 in [threat-model.md](../threat-model.md) cover a stolen token and a secret in a process argument. AGENTS.md forbids a secret in a process argument. The shell history, the process list and `/proc/<pid>/cmdline` all show a command-line argument.

The GitLab runner registration is the reference model. An administrator makes a one-time token in the web interface. The host runs one command, and the agent keeps its identity.

This record is a deliberate exception to the AGENTS.md rule, for one secret only. The enrollment token is single use and the first successful registration consumes it. The default TTL is one hour (FR-ID-01) and an Admin can revoke the token. After a successful registration the value in the shell history or the process list is worthless. The rule stays for every long-lived secret: a repository password, a key and an API token.

## Decision

- The Admin creates a one-time enrollment token in the Web UI, on the "Add host" screen. FR-IN-01 holds the generated command.
- The screen shows the full registration command with the control plane URL and the token: `restichub-agent register --url https://<control-plane> --token <token>`. The screen shows the command one time.
- A person unpacks the agent package on the host and runs this command.
- `--token <token>` is the documented default, as in the GitLab runner model. The option `--token-file <path>` and the standard input are optional, for a script and an unattended run.
- `register` generates the agent key pair. It sends the certificate signing request and the token to the enrollment endpoint (#48, ADR-0026). It stores its identity in the protected configuration directory (FR-AG-04).
- The identity is the agent private key, the agent certificate, the control plane root certificate, the control plane address and the public job signing key.
- Then the person runs `restichub-agent install-service`. The command writes and enables the operating system service. The service is a Windows service as LocalSystem, a systemd unit on Linux and a launchd daemon on macOS (FR-AG-01).
- The MVP does not install an agent over the network. FR-IN-03, FR-IN-04 and FR-IN-05 stay 1.0.

## Consequences

- The token appears in the shell history and in the process list until the registration ends.
- The token is single use with a one-hour default TTL. An Admin can revoke an unused token. The residual window is the time before the first use, or after a failed registration until the TTL ends.
- The operator copies the command once, like the GitLab runner model.
- Each host needs one manual step. The MVP has no fleet installation.
- The registration and the service install are two commands. A person can register a host before the identity serves the service.
- A script and an unattended run use `--token-file` or the standard input.

## Threats

| Threat | Control in this record |
|---|---|
| T21: a token in the shell history or in a process argument | The token is single use with a default TTL of one hour, and an Admin can revoke it (FR-ID-01). A successful registration consumes it. The audit log records the create, the use and the revoke. A used or an expired token is refused (AC-IN-02). The short residual window before the first use is accepted. |

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| No `--token` flag: the standard input or a file only | The token is worthless after the first successful registration. The explicit flag is the GitLab runner model and is simpler for the operator. The residual window is accepted. |
| One command that registers and installs the service | The register step takes the token and the service step needs administrator rights. Two commands keep the secret out of the service configuration. |
| An installation script with the token embedded | The token lands in a file on the host. This is the 1.0 remote installation path (FR-IN-03). |
