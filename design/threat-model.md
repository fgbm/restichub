# Threat model

Status: draft. Date: 2026-10-12. The maintainers review this document before each release (NFR-SEC-02).

## 1. Assets

| Asset | Where it is |
|---|---|
| Backup data | restic repositories |
| Repository passwords and backend credentials | Control plane database, encrypted. Agent memory during a job. |
| Key encryption key (KEK) | File or environment variable on the control plane host |
| Job signing keys (current and next) | Control plane database, wrapped by the KEK (ADR-0026) |
| CA private keys (root and issuing) | Control plane database, wrapped by the KEK (ADR-0026) |
| Agent private keys | Each agent host |
| Hook allow-lists | Each agent host |
| Audit log | Control plane database |

## 2. Trust boundaries

1. Browser or API client and control plane.
2. Agent and control plane.
3. Agent and repository backend.
4. Control plane and PostgreSQL.
5. Tenant and tenant in one installation. The account rows are installation-wide (ADR-0025).

## 3. Threats and controls

| ID | Threat | Control | Residual risk |
|---|---|---|---|
| T1 | A compromised agent reads or deletes a repository during a job. | Secrets only for one job (ADR-0003). Dedicated maintenance agent. Storage controls in NFR-SEC-08. | **Accepted.** An attacker with root on the host during a job can delete the repository if the backend permits deletion. |
| T2 | A compromised control plane executes code on all hosts. | Hook allow-list on agents (ADR-0004). Typed steps without raw arguments (FR-PR-01). | The control plane can still send restic steps, for example `forget` with a short retention. Storage controls in NFR-SEC-08 limit the damage. |
| T3 | An attacker writes to the database and inserts jobs. | Job signatures with a key outside the database (ADR-0005). | None if the signing key stays safe. |
| T4 | Replay of an old job. | Job journal and maximum lifetime (ADR-0020). | A redelivered job returns the stored result. It does not run again. |
| T5 | A database dump discloses secrets. | Envelope encryption with a KEK outside the database (ADR-0017, ADR-0023). | If an attacker gets the KEK and the dump, all secrets are disclosed. |
| T6 | One agent impersonates another agent or another tenant. | Identity only from the mTLS certificate (FR-PR-08). Row-Level Security (ADR-0006, ADR-0023). | None. |
| T7 | A user of one tenant reads data of another tenant. | Application checks, Row-Level Security (ADR-0006, ADR-0023), isolation test AC-TN-01. | Defects in new endpoints. AC-TN-01 fails on unchecked endpoints. |
| T8 | A stolen enrollment token enrolls a false agent. | One-time token with a short TTL. Optional quarantine mode (FR-ID-06). | A false agent can receive jobs of its tenant until an Admin revokes it. |
| T9 | Secrets in process arguments, logs or crash dumps. | Environment variables or pipe only (FR-AG-06). Masking (NFR-SEC-05). Crash dumps disabled (FR-AG-07). | A local administrator can read the environment of a running process. |
| T10 | A malicious update of the agent. | Release key separate from control plane keys (FR-AG-13). | Compromise of the release key. |
| T11 | Denial of service through enrollment. | Rate limit (FR-ID-05). | A distributed attack. Use a reverse proxy with rate limits. |
| T12 | Option injection through step string fields. | Argument construction and validation (ADR-0021, AC-PR-08). | Unknown restic behavior in new step types. |
| T13 | A forged signing key update. | Key update rules (FR-PR-11, AC-PR-09). | The first public key needs a trusted delivery path. |
| T14 | A tenant Admin takes over an account of another tenant. | The account row is installation-wide and separate from the membership (ADR-0025). A tenant Admin cannot reset the password or change the display name of a shared account (HTTP 403 `shared_account`). It cannot attach an existing foreign account or disable the account. | None for an account with no membership in another tenant. |
| T15 | Username enumeration through the account create endpoint. | The create endpoint needs the Admin role and a rate limit of 20 creates per minute for each actor and tenant. Invitations by email arrive in 1.0 (ADR-0025). | **Accepted.** The answer HTTP 409 `username_taken` reveals an existing name. A tenant Admin can probe names. |
| T16 | Session fixation on the tenant switch. | The switch deletes the old session and rotates the cookie. The new session token is 256 random bits (ADR-0023, ADR-0025). | None. |
| T17 | A revoked agent stays connected or reconnects. | The control plane checks the registry status and the certificate serial at each handshake and stream open (ADR-0026, FR-ID-04). A revoke invalidates the cache and closes the stream at once. | A stale cache entry delays the close by the cache lifetime. |
| T18 | A CA private key or a job signing key in a database dump. | The KEK wraps each key outside the database (ADR-0017, ADR-0023, ADR-0026). | An attacker with the KEK and the dump gets the keys. `rcgen` and `ring` keep some private key bytes in the heap in a buffer that they do not zeroize. The control plane holds the plaintext only for the current issuing CA. It drops every other copy at load. It refuses a core dump on Linux (ADR-0026). |
| T19 | An attacker swaps a stored key blob. | The AAD binds the ciphertext to the tenant and the key role. The start check binds the key to the certificate of its row (ADR-0026). | None. |
| T20 | A tampered restic binary on a host. | The agent build embeds the expected SHA-256 of the bundled restic and the agent refuses a different binary. The control plane cannot set the restic path (ADR-0031). | **Accepted.** An attacker with write access to the agent directory and to the agent process can replace the binary and patch the check. The check raises the cost of an accidental or a partial swap. |
| T21 | A token in the shell history or in a process argument. | The token is single use with a short TTL and an Admin can revoke it (FR-ID-01). A successful registration consumes it. The audit log records the create, the use and the revoke. | **Accepted.** The token stays in the shell history and in the process list until the registration ends. Before the first use, or after a failed registration until the TTL ends, the value is live. |
| T22 | A Platform Admin reads the tenant data of an organization through the platform right. | The right is an account attribute and gives no tenant data access (ADR-0029). A platform operation runs in the platform scope; the tenant data needs a membership and its role (ADR-0006, ADR-0025). AC-AU-17 checks it. | None for the right alone. The operator grants a membership on purpose. |
| T23 | A Platform Admin adds a membership for itself and hides the change. | The write needs the Platform Admin right, and it is an audit record in the platform audit and in the target organization's audit (ADR-0029). | A Platform Admin can add itself to any active organization. The operator reads the platform audit. |
| T24 | A user-facing list shows the internal platform tenant. | The organization list, the switcher and the tenant API exclude the platform tenant (ADR-0029). AC-TN-08 checks it. | None. |
| T25 | An organization Admin takes over an account that holds the Platform Admin right, also when the account is a member of that organization only. | A tenant-scope write on such an account is refused (ADR-0029): the password, the display name, the disable, the role change and the membership removal. The guard is in the administrator functions and in a trigger on `memberships`. AC-AU-07 checks it. | None. |

## 4. Recommendations for operators

1. Turn on bucket versioning and lifecycle rules on S3 repositories.
2. Give each host group its own backend credentials.
3. Run the maintenance agent on a dedicated host.
4. Copy important repositories (`restic copy`) to a second repository that agents cannot access.
5. Keep the KEK outside the database backup. It wraps the CA keys, the job signing keys and every secret. Without the KEK, all of them are lost.
