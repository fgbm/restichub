# ADR-0002: The control plane owns all scheduling

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-AG-02, FR-AG-03, FR-SC-01 to FR-SC-11

## Context

Tools such as resticprofile and Backrest keep schedules on each host. A central view of the fleet then needs data collection from each host. Conflicts between operations on a shared repository are hard to prevent. Exclusive restic operations (`prune`, `check`, `forget`) cannot run at the same time as other operations on the same repository.

## Decision

The control plane makes all decisions about when a job runs. The agent keeps no schedules, profiles or retention policies. The agent has no autonomous mode. It starts no action without a command from the control plane, except connection, heartbeat and delivery of results.

## Consequences

- The control plane can serialize operations on each repository (FR-SC-04). This prevents lock conflicts.
- One place shows the state of all backups.
- If the control plane is not available, no new backups start. ADR-0018 covers this risk.
- Laptops and hosts that are often offline need a missed run policy (FR-SC-07).

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Local schedules on agents, with central reporting | No central control of lock conflicts. Configuration drift. |
| Signed schedule lease that the agent can run offline | It makes the agent more complex and conflicts with the main principle. We can examine it again after 1.0. |
