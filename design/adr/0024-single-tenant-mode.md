# ADR-0024: Single-tenant mode

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-TN-01..06

## Context

Most installations serve one organization. The operator of such an installation does not need tenants. The operator wants no tenant in the user interface and in the API. The operator wants the option to add tenants later without a data migration.

The multi-tenancy decision (ADR-0006) is expensive to change after the first migration. The mode must therefore not change the storage model.

## Decision

- The storage model does not change. Each table keeps `tenant_id`. Row-Level Security stays. ADR-0006 stays in force. The mode changes only the configuration, the API surface and the user interface.
- The setting `tenancy.mode` (environment variable `RESTICHUB_TENANCY_MODE`) has the values `single` and `multi`. The default is `single`.
- In single mode, the initial setup creates the tenant `default` and its first Admin. It creates no Platform Admin account. Sign-in does not ask for a tenant and uses `default`. The platform endpoints (`/api/v1/platform/*`) return HTTP 404 `not_found`. The user interface hides the platform sections and the tenant field. The enrollment tokens and the agents belong to `default`. `GET /api/v1/setup/status` and a new public `GET /api/v1/instance` report the mode, so the user interface can adapt.
- To switch from single mode to multi mode, the operator changes the setting and restarts the control plane. If the platform tenant has no user, the control plane issues a one-time setup token for the Platform Admin. The rules of ADR-0023 apply. The tenant `default` continues as a normal tenant. No data migration occurs.
- To switch from multi mode to single mode, the control plane refuses to start unless exactly one non-platform tenant is active. The tenant must be named `default`, or the operator sets `tenancy.default_tenant` to its name. The operator must delete the other tenants first.
- Security: single mode does not weaken the isolation. Row-Level Security and the tenant checks run the same way.

## Consequences

- The installation for one organization is simple to operate. The operator sees no tenant.
- The switch between the modes needs a restart only. The data of the tenant `default` stays available.
- Single mode is the default. Most installations start without tenant terms.
- The tests must cover both modes and the switch (AC-TN-06).
- The configuration validation at start has one more rule. The rule applies in multi mode only.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Remove `tenant_id` in single mode | It violates ADR-0006 and it blocks the later switch to multi mode. |
| A separate build for single mode | It doubles the build and the test matrix. The two builds then drift apart. |
