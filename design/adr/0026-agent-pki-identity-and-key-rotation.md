# ADR-0026: Agent PKI, identity and key rotation

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-ID-01..05, FR-ID-07, FR-ID-08, FR-PR-03, FR-PR-04, FR-PR-05, FR-PR-08, FR-PR-11, NFR-SEC-01, NFR-SEC-04

## Context

ADR-0008 fixes the agent protocol: Protocol Buffers over gRPC with mTLS and an own CA. ADR-0006 fixes one CA and one job signing key for all tenants. ADR-0017 and ADR-0023 fix envelope encryption and the KEK.

The protocol epic needs these decisions before the code. The decisions are the CA hierarchy, the agent identity, the revocation method, the enrollment transport and the job signing key rotation. This record fixes them once. Later work then does not change the certificate format, the identity or the rotation protocol.

The P4 spike ([p4-stream-load.md](../spikes/p4-stream-load.md)) gives the reconnect numbers for 2000 agents.

## Decision

### Certificate authorities

- One installation root CA has a lifetime of 10 years. One issuing CA is signed by the root and has a lifetime of 2 years. The agents trust the root CA.
- The control plane wraps each CA private key with the KEK (ADR-0017, ADR-0023). The AAD format is in section CA key format. The control plane stores both keys in the platform tenant (ADR-0006).
- The issuing CA rotates with an overlap. The root CA signs a new issuing CA. The control plane accepts both issuing CAs until the old one expires. Each agent gets a certificate from the new issuing CA at its next rotation.
- The root CA rotation is a manual procedure. ResticHub 1.0 documents the procedure.

### Revocation

- There is no CRL and no OCSP. The control plane is the only verifier of an agent certificate.
- At each TLS handshake and each stream open the control plane checks the agent registry status and the certificate serial. The check uses a cache. A revoke invalidates the cache entry.
- A revoke closes an open stream at once (FR-ID-04) and refuses a new connection.
- The revocation of the control plane server certificate is out of scope for the MVP. The documentation states this limit.

### Identity

- The agent certificate carries the SAN URI `restichub://tenant/<tenant_uuid>/agent/<agent_uuid>`. The common name (CN) holds the agent UUID and is informational only.
- The control plane accepts a stream only in these cases. The URI parses. The registry row with that agent ID exists, is active, belongs to that tenant and lists that certificate serial.
- The tenant of each agent message comes from this identity. The tenant never comes from a message field (FR-PR-08, AC-TN-02).

### Enrollment transport

- Enrollment is HTTPS `POST /agent/v1/enroll` on the agent listener. The listener serves the gRPC stream and the enrollment endpoint on one port.
- The listener requests a client certificate but does not require one. Each handler enforces its own rule. The enrollment handler accepts a request without a client certificate.
- The endpoint has a rate limit. The 11th request from one IP address in one minute gets HTTP 429 (AC-ID-05).
- The contract has no `EnrollmentService`. The token, the CSR and the response are JSON fields, not Protocol Buffers messages.

### Certificate lifetime and rotation

- The certificate lifetime comes from FR-ID-03: 30 days by default. The agent rotates the certificate at two thirds of the lifetime.
- The old certificate stays valid until the agent confirms the new certificate over the stream. The control plane then refuses the old certificate serial.
- The agent sends the certificate signing request over the existing stream (`CertificateSigningRequest`, `CertificateIssued`). The control plane checks that the current certificate owns the request.

### Reconnect

- The agent reconnects with exponential backoff and full jitter. The base is 1 second and the maximum is 60 seconds. These values cover a thundering herd of 2000 agents (P4).

### Job signing keys

- One key set serves all tenants (ADR-0005, ADR-0006). The set holds the current key and the next key.
- The control plane sends the key set to the agent over the stream.
- Each job names its `signer_key_id`. The agent verifies the job with that key.
- A rotation retires the old key when the agent that holds it acknowledged the new set. A rotation refuses while an earlier rotation still holds its overlap. The control plane retires the old key at most 7 days after the rotation starts, also when an agent did not answer.
- An agent that was offline longer re-syncs the key set at its reconnect.
- The control plane wraps each job signing key with the KEK, in the manner of a CA key.
- Re-sync of an agent more than one rotation behind (2026-10-13): the control plane keeps every retired key row. It holds the whole chain of keys. It sends the chain of `KeyUpdate` messages, one step per rotation. Each message is signed by the key that the agent already holds. The agent reaches the current key in a few steps. The chain uses the retained rows, so a re-enrollment is not needed. The data side is the retained rows; the delivery and the acknowledgement are issue #50.

### CA key format

- A CA private key uses AES-256-GCM with a random 96-bit nonce.
- The associated data is the bytes `restichub-ca-key-v1\0`, then the 16 tenant ID bytes. Then come the 4 length bytes of the CA role in big-endian order and the CA role bytes. The CA role is `root` or `issuing`.
- A wrapped job signing key uses the same form. The associated data is the bytes `restichub-signing-key-v1\0`, then the 16 tenant ID bytes, then the 16 key ID bytes. Then come the 4 length bytes of the key role in big-endian order and the key role bytes. The key role is `current` or `next`.
- The tenant ID, the key ID and the role in the associated data bind the ciphertext to one key row. The start check refuses a swapped blob. The unwrapped key must match the `public_key` of the row, and a CA key must match the certificate of the row (T19).
- The key ID in the associated data (2026-10-13): the review of #49 found a gap. The tenant and the role alone do not bind a blob to its row. The associated data now carries the 16 key ID bytes. The `signing_keys.public_key` column holds the public key of the wrapped key. The start refuses a key that does not match its row.

## Consequences

- The installation has two CA levels. A root CA compromise is not necessary for routine operation. Section Alternatives explains the rejected one-level and per-tenant models.
- A revoke takes effect at the next handshake and on an open stream at once. No CRL distribution and no OCSP responder exist.
- The identity is in the certificate URI. The registry lookup binds one certificate to one agent and one tenant.
- Enrollment is an HTTPS JSON endpoint. The install scripts use one port for the stream and the enrollment.
- The key set rotation protects an agent from a lost key. The 7-day bound removes the old key also when an agent stays offline.
- The control plane does not verify a revoked certificate by itself. The registry is the single source of truth. A stale registry entry can keep a revoked agent connected until the cache expires.

## Threats

This record answers the threats below. [threat-model.md](../threat-model.md) holds the rows.

| Threat | Control in this record |
|---|---|
| T6: one agent impersonates another agent or another tenant | The SAN URI identity and the registry check bind a certificate to one agent and one tenant. The tenant never comes from a message field. |
| T8: a stolen enrollment token enrolls a false agent | The one-time token, the short TTL and the enrollment rate limit. |
| T13: a forged signing key update | The signed key set and the `signer_key_id` per job. |
| T17: a revoked agent stays connected | The registry check at each handshake and stream open. A revoke invalidates the cache and closes the stream at once. |
| T18: a CA or signing key in a database dump | The KEK wraps each key outside the database (ADR-0017, ADR-0023). |
| T19: a swapped stored key blob | The AAD binds the ciphertext to the tenant and the role. |

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| A CRL or OCSP | The control plane is the only verifier. A CRL adds a distribution path and an expiry. The registry check is simpler and is already the source of truth. |
| One CA for each tenant (a per-tenant sub-CA) | More key management for the same isolation. The registry check enforces the tenant binding. |
| gRPC enrollment | AC-ID-05 asks for HTTP 429 on the enrollment endpoint. An HTTPS endpoint gives the answer directly and needs a simpler install script. |
| The tenant in a message field | A message field is not trusted. The certificate identity is the only source (FR-PR-08). |
| A one-level CA without an issuing CA | The root key would sign each agent certificate. The root key can stay offline with a separate issuing CA. |

## Implementation notes

These notes refine the decision. Issue #45 puts them into the code. The decision itself does not change.

- A CA key and the control plane server key use ECDSA P-256 (`PKCS_ECDSA_P256_SHA256`). The rustls and tonic stack, `rcgen` with `ring` and the X.509 parser support it for a client and a server certificate. Ed25519 is not used: a TLS 1.2 stack and some verifiers reject it.
- Every certificate carries a random 16-byte serial number with the top bit clear. The same key pair therefore signs certificates with different serials.
- The root certificate has a path length of 1 and each issuing certificate a path length of 0. The agents trust the root certificate.
- The control plane accepts a certificate signing request with an ECDSA P-256 public key only. It refuses a request with an RSA or a P-384 key. It refuses a request with a CA flag or with a subject alternative name that it does not set.
- At load the control plane checks each row. It parses the certificate and unwraps the key. The public key of the key must equal the key of the certificate. The basic constraints must match the role. Each issuing CA must chain to the root. The validity comes from the certificate, not from the `not_before` and `not_after` columns.
- At start the control plane rotates the issuing CA when less than 60 days remain. A leaf certificate never outlives its issuing CA. The control plane cuts the `not_after` to the issuer.
- Under the advisory lock a rotation re-reads the rows. It returns the newer issuing CA of a concurrent rotation. Two starts therefore add one issuing CA only.
- A start with a root and no issuing CA creates the issuing CA. A start with an expired root refuses with a message and points to the manual root rotation procedure.
- The control plane holds the plaintext key of the current issuing CA in memory, so each issue reuses it. It drops every other plaintext copy at load. `rcgen` and `ring` keep some private key bytes in the heap in a buffer that they do not zeroize. The control plane refuses a core dump on Linux with `PR_SET_DUMPABLE`, and a service unit sets `LimitCORE=0`. This is the accepted residual risk next to T18.
- The manual root CA rotation procedure is in `docs/src/content/docs/operations/root-ca-rotation.md`. It is planned for 1.0.

