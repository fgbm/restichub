# ADR-0008: Agent protocol: Protocol Buffers, gRPC, mTLS, own CA

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-ID-01 to FR-ID-08, FR-PR-01 to FR-PR-09

## Context

The agent keeps a permanent connection to the control plane. The control plane pushes jobs. The agent sends progress, logs and results. Steps must be typed. The protocol must stay backward compatible between minor versions.

## Decision

- The contract is in Protocol Buffers. `buf lint` and `buf breaking` run in CI.
- The transport is a bidirectional gRPC stream over HTTP/2, opened by the agent. We use `tonic` with `rustls`.
- Logs go in the same stream as other messages.
- The agent keeps an application-level heartbeat in addition to HTTP/2 keepalive.
- The control plane has its own CA, made with `rcgen`. The agent generates its key and sends a CSR with a one-time token. The certificate contains the agent ID and the tenant ID.
- Certificates live 30 days by default. The agent rotates the certificate through the existing stream.

## Consequences

- One stream for each agent. At 500 to 2000 agents this is a small load for one process.
- After a control plane restart, all agents connect again. Backoff with jitter and a grace period prevent false alerts.
- Some proxies with TLS inspection break HTTP/2. A fallback transport is on the roadmap (FR-ID-09).
- connect-rust speaks the gRPC protocol. We can replace `tonic` on the server later without a change in agents.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| connect-rust for the agent channel | Version 0.9, before 1.0. `tonic` is more mature. |
| WebSocket with Protocol Buffers | We must write framing, flow control and errors ourselves. |
| Polling over HTTPS | Slower start of jobs (NFR-PERF-02) or more load. |
| NATS or MQTT | An extra component with its own authentication model. |
| step-ca or SPIRE | Extra components. `rcgen` covers the necessary functions. |
