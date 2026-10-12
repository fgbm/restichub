# ADR-0018: High availability is deferred

- Status: accepted
- Date: 2026-10-12
- Requirements: D3, NFR-REL-05

## Context

Agents have no autonomous mode (ADR-0002). If the control plane is not available, no new jobs start. High availability needs more than one control plane instance, coordination and a replicated database. This work is not necessary for the MVP.

## Decision

The MVP runs one control plane instance. The design keeps high availability possible:

- No state outside PostgreSQL.
- Idempotent job creation with the key `(plan_id, fire_time)`.
- Leases on jobs, so that another instance can take a job after a failure.
- Agents reconnect with backoff and can connect to any instance.

## Consequences

- During a control plane outage, scheduled jobs wait. The missed run policy applies after the recovery.
- The documentation gives the recovery procedure for the control plane.
- An alert about the control plane itself must come from an external monitor.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Cluster of three instances in the MVP | Large scope for a small benefit at the start. |
