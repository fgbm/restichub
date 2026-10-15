# ADR-0005: Signed jobs with a dedicated signing key

- Status: partly superseded by ADR-0020 (replay protection)
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

## Amendment, 2026-10-15

The implementation differs from the text above in three points. The decision is not rewritten. A later ADR records the change.

- The CP signs a job at dispatch from the state of the database.
- The signing key is not in a file. It is in the database, wrapped by the KEK (ADR-0026).
- For this reason, "Database access without the signing key does not let an attacker forge jobs" is not true. An attacker who writes plans or profiles gets a job that the CP signs. An attacker who holds the KEK and a database dump also gets the key.

Job signing protects against a network attacker, a replayed or altered job, and a CP without the key. It does not protect against write access to the database. Database permissions and the audit log are the controls. See T3 in the [threat model](../threat-model.md) and FR-PR-04. A separate signing service is a later option.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Only mTLS | No protection against database tampering and replay. |
| One signing key for each tenant | More operations work. We can add it after 1.0 without a protocol change. |
