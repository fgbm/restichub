# ADR-0041: Re-registration that keeps the agent ID, and duplicate detection

- Status: accepted
- Date: 2026-10-15
- Requirements: FR-ID-10, FR-ID-11, FR-ID-04, FR-ID-01
- Extends: ADR-0026, ADR-0032

## Context

ADR-0026 and ADR-0032 describe a registration as the creation of a new identity. A host that lost its certificate, or that an operator registers again, then gets a new agent ID. The plans, the maintenance references and the history stay with the old record. A person who registers the same computer twice also gets two records without a warning.

## Decision

### Token bound to a host

- An Admin can issue an enrollment token for an existing active host. The token stores the agent ID (`replaces_agent_id`).
- The enrollment with such a token keeps the agent ID, the hostname, the labels and everything that refers to the agent. It replaces the certificate and the host facts, clears the pending and previous serials, and sets the host to wait for its first connection. The old serial no longer matches, so the stream layer refuses the old certificate and the CP closes the open stream.
- A bound token takes over an identity, so it is stricter than a normal token. Its time to live is one hour at most (`RESTICHUB_REENROLL_TOKEN_MAX_TTL_SECS` changes the limit). A new bound token for a host revokes the earlier unused bound tokens of that host. A successful use revokes the others. A use for a revoked host is refused and audited.
- The wait of a re-registered host counts from the registration time. The alert "host unreachable" does not open before the first connection (FR-ID-10).

### Duplicate detection

- The agent sends a digest of its machine identifier, not the raw value: the HMAC-SHA256 of a fixed application name under the identifier. The CP stores a keyed hash of the digest. The key derives from the KEK and the tenant, so a database dump alone does not confirm that a machine belongs to a tenant.
- The hash is a hint. The CP shows a warning on the hosts that share it. No decision about identity depends on it alone.
- A re-registration writes the hash as the agent reports it, also when the agent reports none.

### Replacement of an old record

- The Admin replaces the old record from the newer record. The CP requires these conditions: both records are active and share the hash, and the new record is newer than the old one. The old record must also have been silent for the online window (`RESTICHUB_REPLACE_ONLINE_WINDOW_SECS`, default 300). Otherwise the CP refuses the request.
- In one transaction, the CP moves the plans, the maintenance references, the queued jobs and the labels to the new record. Then it revokes the old record.
- After the commit the CP cancels the running jobs of the old record and closes its stream, as for a revoke (FR-ID-04, AC-ID-04). The audit log has `agent_revoke` with the reason `replaced` and `agent_replace`.

## Consequences

- A KEK rotation changes the derived key, so the duplicate warning ignores the older hashes until the hosts register again. The warning is a hint, so this is acceptable.
- Cloned virtual machines that share a machine identifier show a warning. The Admin sees the warning and decides. The silent-old-record rule and the direction rule limit the damage of a wrong decision.
