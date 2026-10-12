# ADR-0005: Signed jobs with a dedicated signing key

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-PR-03, FR-PR-04, NFR-SEC-04

## Context

mTLS proves that the agent talks to the control plane. It does not prove that the control plane created the job on purpose. An attacker with write access to the database could insert jobs. A replayed job could run a destructive step again.

## Decision

The control plane signs each job with an Ed25519 key. The signature covers `job_id`, `agent_id`, `tenant_id`, expiry time, nonce and a monotonic sequence number for each agent. The agent verifies the signature and rejects a job with a different addressee, an expired time or a replay.

The signing key is separate from the TLS key and from the database. In the MVP it is a file. Later it can be in an HSM or a KMS. One signing key serves all tenants (ADR-0006).

The signature covers the serialized bytes of the job. The agent verifies the bytes before it decodes them.

## Consequences

- Database access without the signing key does not let an attacker forge jobs.
- The backup of the control plane must include the signing key (NFR-REL-04).
- Key rotation needs a protocol message that delivers the new public key, signed by the old key.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Only mTLS | No protection against database tampering and replay. |
| One signing key for each tenant | More operations work. We can add it after 1.0 without a protocol change. |
