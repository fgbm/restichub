# ADR-0020: Replay protection with a job journal

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-AG-04, FR-AG-12, FR-PR-03, FR-PR-06

## Context

ADR-0005 uses a monotonic sequence number and a nonce for replay protection. A review found four problems.

- The agent receives the same job again after a lost acknowledgement (FR-PR-06). The agent rejects the job as a replay. Then FR-AG-12 cannot return the stored result.
- Concurrent jobs can arrive out of order. A strictly increasing sequence number rejects a valid job.
- The agent did not check the nonce.
- FR-AG-04 gave the agent no place to store the last sequence number. A restart reset the protection.

## Decision

- The signed `Job` contains `job_id` (a UUID), `agent_id`, `tenant_id`, `issued_at` and `expires_at`. The `Job` contains no sequence number and no nonce.
- The agent rejects a job in these cases:
  - The difference between `expires_at` and `issued_at` is more than 24 hours.
  - `issued_at` is more than 5 minutes in the future.
  - `now` is equal to or later than `expires_at`.
- The agent keeps a persistent journal of accepted `job_id` values. The journal also holds the final status and the result summary. The journal keeps each entry until `expires_at` plus 5 minutes. The journal contains no secrets.
- A job with a known `job_id` does not run again. The agent returns the stored result (FR-AG-12) or reports that the job is in progress.
- The CP delivers the same signed bytes again after a lease expires.
- The signature input has the domain prefix `restichub-job-v1\0` before the serialized `Job` bytes.

## Consequences

- A redelivered completed job returns the stored result. The job does not run again.
- The journal uses disk space on the agent. The agent removes the old entries.
- The protection survives an agent restart.
- The agent and the CP agree on the job lifetime.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Monotonic sequence number for each agent | Concurrent jobs arrive out of order. A lost acknowledgement rejects a valid redelivery. |
| Nonce with a time window | The agent needs a store for used nonces. The journal gives the same protection and also stores the result. |
