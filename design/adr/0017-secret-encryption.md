# ADR-0017: Envelope encryption of secrets with a pluggable KEK provider

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-RP-03 to FR-RP-05, NFR-SEC-04

## Context

The control plane stores repository passwords and backend credentials for all tenants. A database dump must not disclose them. Many installations do not have HashiCorp Vault.

## Decision

- Each tenant has a data encryption key (DEK). The control plane encrypts secrets with AES-256-GCM and the DEK.
- A key encryption key (KEK) encrypts each DEK. The KEK is outside the database.
- A `KekProvider` trait gives the KEK. MVP: file or environment variable. Later: Vault Transit, OpenBao, cloud KMS.
- The API cannot return a secret value after a user saves it.
- The control plane records each delivery of a secret to an agent in the audit log.

## Consequences

- A database dump without the KEK discloses no secrets.
- Rotation of the KEK re-encrypts only the DEKs, not all secrets.
- The backup of the control plane must include the KEK (NFR-REL-04). Without the KEK, the secrets are lost.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Vault as a mandatory dependency | An extra component for small installations. |
| One key for all secrets | Rotation and tenant isolation are harder. |
