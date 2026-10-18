---
title: Security for operators
description: The residual risk of direct repository access, the controls that reduce it, and a summary of the threat model.
---

This page tells you what ResticHub protects, what it cannot protect, and what you do to reduce the remaining risk. The full threat model is in the `design` directory of the repository.

## Residual risk of direct repository access

An agent reads and writes the repository directly. It does not send the backup data through the control plane. For this reason the control plane sends the repository password and the backend credentials to the agent for the duration of one job. The agent does not store them.

During a job, a person with root access to the host can use the password. That person can delete the snapshots and, if the backend permits, the data of the repository. ResticHub accepts this risk. It cannot remove the risk, because restic needs the password to write.

You reduce the risk with storage-side controls. The controls below limit what a compromised host can destroy.

## Controls

Apply all controls that your storage supports.

1. **Bucket versioning.** Turn on versioning on an S3 repository. A deleted object stays as an older version.
2. **Lifecycle rules.** Set a rule that keeps old versions for a fixed time. A deletion then does not remove the data at once.
3. **Separate credentials for each host group.** Create backend credentials for each group of hosts. A compromised host reaches only the repository of its group.
4. **A dedicated maintenance host.** Only the maintenance host runs `forget`, `prune` and `check`. Use a dedicated host for this role. See [Repositories](/restichub/concepts/repositories/). Do not use a host with user software.
5. **A secondary copy.** Copy important repositories with `restic copy` to a repository that no agent can reach. Run the copy from a host that only you control.

## Other actions

- Keep the key encryption key (KEK) outside the database backup. The KEK protects the repository passwords, the job signing keys and the keys of the certificate authority. Without it, a restored database holds no usable secret.
- Copy the repository password outside ResticHub. If you lose the control plane, you cannot open the snapshots without the password.
- Turn on the trust mode for server scripts only on a host that you accept to control from the control plane. See [Server scripts](/restichub/guides/scripts/).
- Put a reverse proxy with rate limits in front of the enrollment endpoint. See [Agent enrollment rate limit](/restichub/operations/enrollment-rate-limit/).
- Revoke a host that you do not trust. See [Agent revocation](/restichub/guides/agent-revocation/).

## Summary of the threat model

| Threat | What protects you | What stays |
|---|---|---|
| A compromised agent reads or deletes a repository. | Secrets exist for one job only. Storage controls. | You accept the risk. Use the controls above. |
| A compromised control plane sends harmful steps. | Steps are typed. A restore writes only below a restore directory. A script runs only where the trust mode is on. | The control plane can still send restic steps, such as `forget` with a short retention. A restore below a root that covers system or user files, such as `/` or `/home`, can lead to code execution. Set narrow restore directories. |
| A network attacker alters or replays a job. A control plane without the signing key sends a job. | The control plane signs each job. The agent checks the signature, the addressee, the expiry time and the replay journal. | Nothing known. |
| Someone writes to the database and adds jobs. | Database permissions and the audit log. The job signature does not help here. The control plane signs each job from the state of the database. The signing key is in the database, wrapped by the KEK. | An attacker who can write to the database, or who has the KEK, makes the control plane send any typed step. Restrict the access to the database. |
| Someone copies a database dump. | Envelope encryption with a KEK outside the database. | If the KEK and the dump leak together, all secrets leak. |
| One agent acts as another agent or tenant. | The identity comes from the mTLS certificate. Row-level security separates the tenants. | Nothing known. |
| Someone steals an enrollment token. | A token works one time and expires. | A false agent gets the jobs of its tenant until you revoke it. |
| Someone floods the enrollment endpoint. | A rate limit for each address and for each token. | A distributed attack. Use a reverse proxy. |

## Related pages

- [Job signing key compromise](/restichub/operations/job-signing-key-compromise/)
- [Agent certificates and enrollment](/restichub/guides/agent-certificates/)
