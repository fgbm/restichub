# ADR-0050: The closing of a one-time job alert

- Status: accepted
- Date: 2026-10-21
- Requirements: FR-MN-03, AC-MN-02
- Amends: the job alert rule in [ADR-0036](0036-alerts-and-notification-channels.md) (section "Alert kinds and state")

## Context

ADR-0036 gives a job alert the subject of the latest finished job. The subject is the plan, or the repository and the job kind, or the agent and the job kind. The alert stays open while that latest job failed. The next succeeded job of the subject closes it.

A job without a plan runs once. A restore answers one request of a person. A `check`, a `forget` and a `prune` come from the maintenance schedule of the repository (ADR-0034). The next success of such a kind can be weeks away, and after a failed restore it can never come. The alert then stayed open until the failed job fell out of the 30-day window of the finished jobs that one pass reads. That close sent no message, so the operator kept a red alert for a month and then lost the alert without a recovery notification.

The alert must close in a time that an operator can predict, and the close must send the recovery message that ADR-0036 defines.

## Decision

- A job kind is one-time when it is `restore`, `check`, `forget` or `prune`.
- A job alert (`job_failed`, `job_dead_letter` or `job_warnings`) of a one-time kind closes in two ways. The next succeeded job of the same kind on the same repository closes it, as ADR-0036 says. Without such a job, the alert closes 7 days after the finish time of the failed job.
- The close because of the time sends the recovery notification of ADR-0036. The condition is gone: no later job of the kind holds it.
- A job alert of a plan, and a job alert of any other kind, does not change. The next succeeded job of the subject closes it.
- The 7 days are a constant of the control plane for the MVP. A later change makes the value settable for each tenant.

## Consequences

- An operator sees a failed restore for 7 days at most, and the recovery message tells when the alert closed.
- A maintenance failure that no later run repeats closes after 7 days. A repository that no schedule runs for does not hold an alert for a month.
- A `job_failed` alert of a one-time kind can close while the cause stays. The operator must read the alert, not only wait for its close.
- The time of the close is one constant for every tenant. A tenant with a monthly check sees the alert of a failed check close after 7 days, before the next check.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| A manual Resolve action only | An operator who is on holiday leaves the alert open, and the action needs a route, a right, an audit record and a button. It is a later addition. |
| Close at the next success of any job of the repository | A backup runs next to the conditions of the other kinds, so its success says nothing about a failed restore. |
| Close when the failed job leaves the 30-day window | The close is late and silent, so an operator cannot tell a real recovery from a forgotten alert. |
| A time that follows the schedule of the maintenance task | A restore has no schedule, and the alert would wait for the next monthly check. One constant is enough for the MVP. |
