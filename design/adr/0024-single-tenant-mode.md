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

## Implementation notes

These notes refine the decision. The decision itself does not change. Issue #30 puts them into the code.

- The setting `tenancy.mode` (environment variable `RESTICHUB_TENANCY_MODE`) has the values `single` and `multi`. The default is `single`. The setting `tenancy.default_tenant` (environment variable `RESTICHUB_TENANCY_DEFAULT_TENANT`) names the tenant of single mode. The default is `default`. The name obeys the tenant name rule, so the reserved name `platform` is refused. The control plane validates both settings at start.
- The mode lives in the process configuration only. It changes no table, no policy and no query. Row-Level Security and the tenant checks run in the same way in both modes.
- In single mode every route under `/api/v1/platform/*` answers HTTP 404 `not_found`. One guard of the router decides this, and it reads the access rule of the route table. The guard runs before the handler, so an unauthenticated request gets the same 404 and the mode reveals no platform route.
- The setup endpoint `POST /api/v1/setup` needs the one-time setup token of ADR-0023 in both modes. The token protects the tenant that the setup creates: the tenant `default` in single mode, the platform tenant in multi mode. The request field `setup_token` is optional in the schema; a missing value gives HTTP 400 `invalid_argument` with the field `setup_token`. The endpoint keeps the CSRF header and the per-IP rate limit of sign-in.
- In single mode the setup endpoint creates the tenant `default` through `tenants::create_with_bootstrap`. The tenant, its key row, its first Admin and the audit records commit in one transaction. The token belongs to the platform tenant, which always exists. Each step of the endpoint opens one transaction and releases it before the next step. The setup therefore never holds a pooled connection while it waits for another. The check "does the tenant exist?" runs before the lock, so a repeated call gets HTTP 409 at once. The unique index on the tenant name makes exactly one of two concurrent setup calls win.
- A successful setup marks the used token and every other unused token of the platform tenant as used. One transaction marks the two.
- In single mode sign-in ignores the `tenant` field and uses `tenancy.default_tenant`. In multi mode the field is required; a missing value gives HTTP 400 `invalid_argument` with the field `tenant`.
- In single mode a session or an API token of the platform tenant cannot authenticate: the control plane answers HTTP 401 `unauthenticated`. Single mode shows no platform tenant in the user interface and in the API. A Platform Admin that still exists therefore cannot sign in either. The rows stay. A later switch to multi mode makes the account usable again.
- The start checks the mode. Single mode refuses to start unless the database holds exactly one active non-platform tenant, and the name of that tenant is `tenancy.default_tenant`. An installation with no non-platform tenant is a fresh install, so the setup creates the tenant. A suspended tenant and a deleted tenant do not block the switch, because only an active tenant serves requests. The operator must suspend or delete the other active tenants first. The error names the problem.
- At start the control plane issues a one-time setup token when the setup is required. In single mode the setup is required while the tenant `default` has no user. In multi mode it is required while the platform tenant has no user. The rules of ADR-0023 apply. The value is printed once at WARN level.
- The switch from single mode to multi mode needs no data change. The tenant `default` continues as a normal tenant.
- The endpoint `GET /api/v1/instance` is public. It reports `tenancy_mode` and `setup_required`. It holds no secret and no version detail. `GET /api/v1/setup/status` reports the mode too.
- The user interface reads `GET /api/v1/instance` at load. Single mode hides the tenant field of the sign-in screen and the platform navigation. The setup screen shows the setup token field in both modes, with the hint to read the value from the control plane log.
