# ADR-0010: Job queue and scheduler on PostgreSQL

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-SC-01 to FR-SC-09, D8

## Context

The queue must do more than first-in, first-out. It must hold exclusive operations as barriers for each repository, apply limits for each agent and repository, apply missed run policies and use leases. Generic queue libraries do not model these rules. apalis-postgres 1.0 was published on 2026-10-12. pgmq needs a PostgreSQL extension.

## Decision

- We write the queue on PostgreSQL with `sqlx`: `SELECT ... FOR UPDATE SKIP LOCKED`, lease column, attempt counter, `run_at` for backoff, dead-letter state.
- Repository barriers and limits are part of the selection query.
- The scheduler uses `croner` to calculate the next fire time. Each plan stores its time zone and `next_run_at`.
- Job creation is idempotent: unique key `(plan_id, fire_time)`. Two scheduler instances cannot create the same job.
- In a DST gap, a fixed-time job runs once, immediately after the clock change. In a DST overlap, a fixed-time job runs once, at the first occurrence. An interval expression, such as `0 * * * *` or `*/15`, names no wall-clock time, so it runs in both halves of the overlap.

## Consequences

- No extra infrastructure.
- We own and test the queue code.
- The unique key prepares the scheduler for high availability without leader election.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| apalis-postgres | Published on the day of the decision. It does not model repository barriers. |
| pgmq | Needs a PostgreSQL extension. Less portable. |
| tokio-cron-scheduler | Keeps jobs in memory. We need state in the database. |
| Redis Streams | An extra component. |
