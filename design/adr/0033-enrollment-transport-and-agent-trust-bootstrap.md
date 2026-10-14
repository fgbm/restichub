# ADR-0033: Enrollment transport and agent trust bootstrap

- Status: accepted
- Date: 2026-10-14
- Requirements: FR-ID-01, FR-ID-02, FR-ID-07, FR-PR-04, NFR-SEC-01, NFR-SEC-04

## Context

[ADR-0026](0026-agent-pki-identity-and-key-rotation.md) puts the enrollment endpoint on the agent listener. The listener serves the enrollment and the gRPC stream on one port. The listener presents a server certificate from the installation issuing CA.

The agent trusts a certificate authority in two different places:

- The enrollment transport. The `register` command of [ADR-0032](0032-manual-agent-registration-and-service-install.md) uses the system trust store, as the screen command `restichub-agent register --url https://<control-plane> --token <token>` shows. The installation CA is not in a system trust store, so the enrollment on the agent listener fails without an extra `--ca-file` option.
- The stream. The agent dials the listener address from the enrollment response and must trust the same installation CA.

The operator needs one command with no extra file, and the agent must trust the installation CA for the stream.

## Decision

- The control plane serves the enrollment endpoint `POST /agent/v1/enroll` on two origins: the agent listener and the public web origin. The public web origin is the HTTPS address behind the reverse proxy, with a publicly trusted certificate. Both origins use one handler and the same rate limits (FR-ID-05, AC-ID-05).
- On the web origin the client address comes from the `X-Forwarded-For` header through the trusted proxy list (ADR-0023). On the agent listener the client address is the socket address only. The agent listener has no proxy in front (ADR-0026).
- Both origins keep the request body limit of the route. A body above the limit gives HTTP 413.
- The enrollment response carries the installation CA chain, the issuing CA and the root CA. The agent stores the root CA and pins it for the stream. The stream never uses the system trust store.
- The enrollment response carries the listener address in `grpc_endpoint`. The agent dials this address and verifies the listener certificate against the pinned root CA. The listener certificate carries the endpoint host as a subject alternative name.
- The agent needs no `--ca-file` option for the default path. The system trust store verifies the web origin at enrollment; the pinned root CA verifies the listener at the stream.

## Consequences

- One command enrolls an agent against the public web address. The operator adds no file.
- The enrollment handler runs on two origins. A change to the handler reaches both.
- The web origin is a second entry point to the enrollment. It needs the rate limit of FR-ID-05 and the audit record, so the two origins cannot diverge.
- The agent holds the installation root CA after the enrollment. A stream to a listener with a certificate from another CA fails.
- The reverse proxy forwards `POST /agent/v1/enroll` to the control plane. The reverse proxy does not terminate the agent listener, because the mTLS of the stream must reach the control plane (ADR-0026).

## Threats

This record answers the threats below. [threat-model.md](../threat-model.md) holds the rows.

| Threat | Control in this record |
|---|---|
| T8: a stolen enrollment token enrolls a false agent | The one-time token, the short TTL and the rate limit apply on both origins (FR-ID-01, FR-ID-05). |
| T6: one agent impersonates another agent or another tenant | The pinned root CA and the registry check of ADR-0026 stay the only stream identity. The enrollment transport adds no identity. |

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Enrollment on the agent listener only, with `--ca-file` | The default command needs an extra file. The operator must copy the installation root CA to each host before the registration. |
| Enrollment on the web origin only | The agent listener serves the stream. ADR-0026 keeps the enrollment on the same port as the stream. Two origins cover a listener-only installation and the default public command. |
| A public CA certificate on the agent listener | The listener would need a public certificate for each installation. The installation CA keeps one trust anchor for the whole installation. |
| The agent trusts the system roots for the stream | The listener presents a certificate from the installation CA. A system root cannot verify it. |
