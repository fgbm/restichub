# ADR-0009: Public API: REST, OpenAPI and Server-Sent Events

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-API-01, FR-UI-02, AC-UI-01

## Context

The web UI and external tools (Ansible, Terraform, monitoring) use the same API. The UI must show progress and logs in real time. The UI sends commands and receives events. It does not need a bidirectional channel.

In August 2026 connect-rust joined the ConnectRPC project. It supports Connect, gRPC and gRPC-Web, but it is before version 1.0.

## Decision

- The public API is REST with JSON. We use `axum`.
- `utoipa` generates the OpenAPI specification from the Rust code.
- The TypeScript client for the UI is generated from the specification.
- Real-time events go to the browser through Server-Sent Events.
- The browser session uses an HttpOnly cookie. External tools use API tokens.
- We evaluate ConnectRPC again before 1.0.

## Consequences

- Administrators can use `curl` and common tools.
- One source of truth for the API: the Rust code.
- Server-Sent Events have fewer problems with proxies than WebSocket and reconnect automatically.
- The agent protocol (Protocol Buffers) and the public API (OpenAPI) are two contracts.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| ConnectRPC for the UI and the API | One contract, but connect-rust is before 1.0, and integrators expect REST. |
| GraphQL | More complexity. No need for flexible queries. |
| WebSocket for events | Not necessary for one-way events. More work with proxies and authentication. |
