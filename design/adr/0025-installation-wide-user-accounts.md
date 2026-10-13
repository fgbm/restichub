# ADR-0025: Installation-wide user accounts

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-AU-01..04, FR-TN-01, FR-TN-06

## Context

ResticHub serves one installation with one or more tenants (ADR-0006). The tenant model and the Row-Level Security are fixed. Each table has a `tenant_id`.

Today a user row belongs to one tenant (ADR-0023, section Local authentication). The sign-in page asks for the tenant name and the username (ADR-0024, section Implementation notes). A person that works in two tenants needs two accounts and two passwords.

The operator decided that a person has one account in the installation. The person signs in one time and then switches between the tenants. This record fixes the account model, because a change after the first release is expensive.

## Decision

### The account

- A user account is installation-wide. The username is unique in the installation and the comparison ignores the letter case.
- The account rows live in the platform tenant. The column `tenant_id` holds the platform tenant id, so ADR-0006 stays true.
- An account holds: id, username, password_hash, display_name, disabled, last_used_tenant_id, created_at, updated_at.
- A membership `(tenant_id, user_id, role, disabled)` lives in each tenant under Row-Level Security. A Platform Admin is a membership in the platform tenant.
- The role belongs to the membership, not to the account. One account can be an Admin in one tenant and a Viewer in another.

### Sign-in

- The sign-in page asks for the username and the password only. It never asks for the tenant name. A later sign-in factor, for example TOTP, obeys the same rule.
- The control plane finds the account through a `SECURITY DEFINER` function owned by `restichub_auth`. The function compares the username without the letter case.
- The control plane verifies the password with Argon2id (ADR-0023). If the account is absent, it verifies a dummy hash. The answer and the time are then the same for every unknown name.
- The control plane lists the active memberships of the account in active tenants. The count decides the session:
  - Zero memberships give `invalid_credentials`. This answer is the same as a wrong password.
  - One membership gives a session in the tenant of that membership.
  - Several memberships give a session in the last used tenant of the account. If the account has no last used tenant, the first membership by tenant name gives the session.
- The session row stays tenant-scoped (`tenant_id`, `user_id`). The control plane stores the used tenant on the account.

### Tenant switch

- The endpoint `POST /api/v1/auth/switch` with the body `{"tenant_id": "..."}` moves the session to another tenant.
- The endpoint checks the membership of the caller in the target tenant. A missing membership gives HTTP 403 `forbidden`.
- The endpoint creates a new session in the target tenant, deletes the old session and rotates the session cookie. The new session defeats session fixation.
- The endpoint writes the audit record `session_switch` in the source tenant and in the target tenant. It stores the target tenant as the last used tenant.
- The endpoint `GET /api/v1/auth/tenants` lists the memberships of the caller only. Each item holds the tenant id, the tenant name and the role. It returns no other account.

### Rate limits

- The sign-in rate limit keys on the username (installation-wide) and on the source IP network. The tenant key is gone, because the sign-in has no tenant.
- The resolved identity map keys on the account id.

### User administration

- A tenant Admin creates an account. The endpoint gives HTTP 409 `username_taken` if the name exists in the installation.
- We accept this risk. The create endpoint needs the Admin role and it has a rate limit. Invitations by email in 1.0 remove the enumeration risk (see Alternatives).
- A tenant Admin changes the role of an account in its tenant. It disables or removes the membership in its tenant. Disabling or removing the membership deletes the sessions of the account in that tenant.
- A tenant Admin cannot add an existing account of another tenant to its tenant. Only a Platform Admin adds a membership for an existing account.
- A tenant Admin cannot reset the password or change the display name of an account that has an active membership in another tenant. The answer is HTTP 403 `shared_account`. The user or a Platform Admin can change them.
- A tenant Admin cannot disable the account itself. Only a Platform Admin disables an account.
- A tenant Admin reads the accounts of its members only. It reads them through a `SECURITY DEFINER` function or a view that joins the memberships of the current tenant. It cannot read another account.
- A password change by the user is installation-wide. It ends the sessions of the account in all tenants.

### Audit

- A membership change in a tenant is an audit record in that tenant.
- An account change (create, password reset, disable) is an audit record in the tenant where the actor acts and in the platform audit.

### Single mode

- Single mode uses the same model. The accounts live in the platform tenant and the memberships live in the tenant `default`. The user interface and the API do not change (ADR-0024).
- An API token stays tenant-owned. An API token of the platform tenant keeps the role `platform_admin` only.

### Suspended and deleted tenants

- The sign-in skips the memberships of a suspended or a deleted tenant.
- A switch to a suspended or a deleted tenant is refused.

### Migration

- No release exists yet. New migrations move each user row to the platform tenant and keep its id.
- A membership row stays in its tenant and refers to the account by `user_id`.
- The migration merges nothing. A username that exists in two tenants stops the migration with a clear message. The message names the two tenants and the username.
- The password hash, the display name and the disabled flag do not change.

## Consequences

- One person has one account and one password. The person switches between the tenants after the sign-in.
- The username is unique in the installation. Two tenants cannot use one name. A tenant Admin learns from HTTP 409 `username_taken` that a name exists. We accept this risk.
- The account rows share the platform tenant with the platform administration. A Platform Admin reads and disables accounts.
- The sign-in works with no tenant name. The rate limit is per username, so one name has one bucket in the whole installation.
- The reach of `auth_find_session` does not change. A new function finds the account by username. The function `auth_find_tenant` stays for the tenant name.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| The tenant name on the sign-in page | The operator rejected it. A person in two tenants needs two sign-ins. |
| Per-tenant users with a global lookup by a unique email | No single identity. The password and the account belong to one tenant. |
| A separate identity table without `tenant_id` | It breaks the rule of AGENTS.md and ADR-0006. |
| An account that belongs to no tenant | Every row needs a `tenant_id` (ADR-0006). |
| Invitations by email | Later, in 1.0. They also remove the enumeration risk. |
