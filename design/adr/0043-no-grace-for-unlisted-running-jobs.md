# ADR-0043: No grace for a running job that the hello omits

- Status: accepted
- Date: 2026-10-16
- Requirements: FR-SC-06, FR-AG-10
- Supersedes: the grace rule in ADR-0039 (section "The control plane does not use a wall clock")

## Context

ADR-0039 lets the CP fail a `running` job that the agent does not list in `AgentHello.held_job_ids`. It adds a grace rule: a job that started a short time ago is not failed, because the acknowledgement and the hello can cross. The grace was a time window of two minutes. A restart of the agent in this window left the job `running` until the lost window of 30 minutes ended. A real backup that the agent lost in the first minutes then looked alive.

The grace is not necessary. The agent puts a job in the set of `held_job_ids` before it sends `JobAck`, and the CP moves a job to `running` only after the acknowledgement. So a job that is `running` for the CP is already in the agent set.

## Decision

- When an agent connects, the CP fails each `running` job of that agent that `held_job_ids` does not list. No time window protects the job.
- The result reason is `agent_lost`, as in the loss of an agent that stays away. The repository lock becomes a suspect (FR-SC-06), so the CP runs `unlock` after the wait.
- A job that is `sent` is not touched. The acknowledgement has not arrived, and the normal lease rules apply.

### Why the rule is safe

- The agent adds the job to its running set before it sends `JobAck`. A repeated delivery of a job that already runs gets an acknowledgement, and the job is already in the set. The agent v0.1.0-rc.3 behaves in the same way.
- The agent writes the stored result in the result journal before it releases the job from the running set. The set `held_job_ids` also holds the stored results of the last 24 hours. A job whose result the CP has not accepted yet stays listed.
- A job that the journal shows as in progress after a restart is not listed, and this is correct. Restic dies with the agent: the agent starts it with `PR_SET_PDEATHSIG` on Linux and in a Job Object with `KILL_ON_JOB_CLOSE` on Windows. The job cannot report a result.
- The agent handles the messages of one stream in order. It builds the hello of a new stream after the loop of the old stream ends. So a hello and a `JobAck` of one agent process do not cross.
- An agent without the capability `job-limits-v1` does not list its jobs. The CP does not fail the jobs of such an agent.

## Consequences

- A restart of the agent in the first minutes of a job ends the job at once with the reason `agent_lost`. The user sees the true state, and the retry rules of the plan apply.
- Two processes that share one identity (for example a cloned VM before a replace, ADR-0041) fail the jobs of each other at every reconnect. This is an accepted risk. The duplicate warning of ADR-0041 shows the situation, and the Admin fixes it with a replace.
- A late `JobAck` of a dead process on another CP instance can still move a job to `running` after the new hello. The job then ends at the next reconnect or at the hard ceiling of ADR-0039. This is not changed here.
