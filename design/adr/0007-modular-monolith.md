# ADR-0007: Modular monolith: one control plane binary and PostgreSQL

- Status: accepted
- Date: 2026-10-12
- Requirements: NFR-OPS-01, NFR-CMP-01, NFR-REL-05

## Context

The first idea had many components: API server, scheduler, queue, compiler, secret store, notification bus, Redis or Redis Streams, and a cluster of three instances. A self-hosted open source product must be easy to install and to operate.

## Decision

The control plane is one Rust binary. It contains the API server, gRPC server for agents, scheduler, queue workers, profile compiler, PKI, secret encryption and the embedded web UI. The only external dependency is PostgreSQL.

The code is split into crates with clear boundaries, so that we can move a part to a separate process later.

We do not use Redis, Kafka, NATS or a mandatory Vault in the MVP.

## Consequences

- Installation is one container and one database.
- Fewer parts to monitor and to back up.
- The control plane keeps no state outside PostgreSQL, so high availability stays possible (ADR-0018).

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Microservices | Operations cost without a benefit at the target scale. |
| Message broker (NATS, Redis Streams) | An extra component. PostgreSQL covers the queue at the target scale. |
