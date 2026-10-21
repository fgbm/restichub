# ADR-0050: The closing of a one-time job alert

- Status: accepted
- Date: 2026-10-21
- Requirements: FR-MN-03, FR-SC-06, AC-MN-02
- Amends: the job alert rule in [ADR-0036](0036-alerts-and-notification-channels.md) (section "Alert kinds and state")

## Context

ADR-0036 gives a job alert the subject of the latest finished job. The subject is the plan, or the repository and the job kind, or the agent and the job kind. The alert stays open while that latest job failed. The next succeeded job of the subject closes it.

Some kinds belong to no plan. A person starts a `restore` and a `test_access`. An Administrator starts an `init` and an `unlock_all`. The control plane starts an `unlock` after an agent loss (FR-SC-06). No schedule repeats these kinds. The next job of such a kind can start weeks after the failure, and after a failed `restore` it can start never. The alert then stayed open until the failed job left the 30-day window of the finished jobs that one pass reads. That close sent no message, so the operator saw a failed job for a month, and the alert then went away without a recovery message.

The maintenance schedule repeats `check`, `forget` and `prune` (ADR-0034). The next run of such a kind comes within the period of its task, so ADR-0036 closes the alert of a failed run in time.

The alert must close in a time that an operator can predict, and the close must send the recovery message that ADR-0036 defines.

## Decision

- A job kind is one-time when no schedule repeats it: `restore`, `init`, `test_access`, `unlock` and `unlock_all`.
- `check`, `forget` and `prune` are not one-time. Their alert closes at the next success of the same kind on the same repository, as ADR-0036 says.
- A job alert (`job_failed`, `job_dead_letter` or `job_warnings`) of a one-time kind closes in two ways. The next succeeded job of the same kind on the same repository closes it, as ADR-0036 says. Without such a job, the alert closes 7 days after the finish time of the failed job.
- The close because of the time sends the recovery notification of ADR-0036. The message tells that the alert closed. It does not tell that the fault is repaired.
- A job alert of a plan never closes because of the time. The plan runs the same job again.
- The 7 days are a constant of the control plane for the MVP. A later change makes the value settable for each tenant.

### The subject holds no agent

The owner asked for the close at the next success of the same kind on the same agent and repository. The subject of a job alert of no plan holds the repository and the kind only (ADR-0036). This record keeps that subject.

- The subject has no agent, and an agent in the subject changes the key of every open alert. The first pass after the upgrade would close the open alerts of the old keys without a message.
- One agent runs the kind for the whole repository in ordinary use. The maintenance host runs the `unlock`, and the repository host starts an `init` and an `unlock_all` (ADR-0046). A `test_access` can name another host, and the subject still holds the repository.
- A successful `restore` on one host closes the alert of a failed `restore` on another host of the repository. The operator reads the alert with the repository and the kind, and sees the host of the failed job in it.

## Consequences

- An operator sees a failed `restore`, `init`, `test_access`, `unlock` or `unlock_all` for 7 days at most, and the recovery message tells when the alert closed.
- A maintenance failure that the schedule repeats stays open until the next success of its kind. It never sends a pair of a recovery message and a new open message each week.
- A repository that no schedule runs for does not hold an alert for a month.
- A `job_failed` alert of a one-time kind can close while the cause stays. The operator must read the alert, not only wait for its close.
- The first pass after the upgrade closes every open alert of a one-time kind whose failed job is older than 7 days. It sends a recovery message for each alert and each channel. A repository whose stale lock mark an older version cleared needs a manual `unlock_all`. Its alerts close, and the next `check`, `forget` or `prune` fails on the lock again.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| A manual Resolve action only | An operator who is on holiday leaves the alert open, and the action needs a route, a right, an audit record and a button. It is a later addition. |
| Close at the next success of any job of the repository | A backup runs next to the conditions of the other kinds, so its success says nothing about a failed restore. |
| Close when the failed job leaves the 30-day window | The close is late and silent, so an operator cannot tell a real recovery from a forgotten alert. |
| A time that follows the schedule of the maintenance task | A restore has no schedule, and the alert would wait for the next monthly check. One constant is enough for the MVP. |
| A time for every kind that belongs to no plan | The maintenance schedule repeats `check`, `forget` and `prune`. A weekly prune that keeps failing closes at 7 days, a moment before the next run fails, and the pair of messages repeats each week. Their alert closes at the next success instead. |
| The agent in the subject of a one-time job alert | The key of every open alert changes at the upgrade, so the first pass closes them without a message. |
