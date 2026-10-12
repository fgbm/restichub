# ADR-0006: Multi-tenancy from the first migration

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-TN-01 to FR-TN-05, FR-ID-02, AC-TN-01

## Context

ResticHub must serve more than one organization in one installation, for example a managed service provider. A tenant model added later needs changes in every table, query and API endpoint.

## Decision

- Each object has a `tenant_id` from the first database migration.
- PostgreSQL Row-Level Security adds a second isolation layer below the application checks.
- One certificate authority and one job signing key serve all tenants. The agent certificate contains the tenant ID.
- The control plane takes the tenant of an agent only from its certificate.
- A Platform Admin role manages tenants. Other roles apply in one tenant.

## Consequences

- Isolation tests (AC-TN-01) cover all endpoints from the start.
- Each database query and each session must set the tenant context.
- One CA is simpler to operate. A CA compromise affects all tenants. The CA key is stored like the signing key.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| One installation for each tenant | High operations cost for service providers. |
| One CA for each tenant | Stronger isolation, but more key management. Not necessary now. |
| Tenant model after 1.0 | Expensive migration of the schema and the protocol. |
