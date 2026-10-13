# ADR-0029: The Platform Admin is an account right, not a membership

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-TN-02, FR-AU-04

## Context

ADR-0025 makes the Platform Admin a membership in the platform tenant. The platform tenant holds the id `00000000-0000-7000-8000-000000000000`. The setup creates the first membership there, and the sign-in picks the platform tenant as the session tenant for a Platform Admin.

The operator decided that the Platform Admin right is independent of the organizations. An account belongs to many organizations (ADR-0025). The account holds the Platform Admin right or not. The organization switcher must list real organizations only, so the platform tenant never appears as an organization.

The platform tenant row stays internal. The `ca_keys` table (ADR-0026) and the signing keys of issue #49 reference it by a foreign key, and the platform audit records carry its id. A user-facing list must never show it.

## Decision

### The right

- We add the boolean `platform_admin` to the account row (`users.platform_admin`, default false). It is a right of the account, not a membership.
- The membership roles are Admin, Operator and Viewer only. The role `platform_admin` leaves the membership set. It stays valid for a platform API token in the platform tenant only.
- The platform tenant is never a membership target: a check constraint refuses a membership with `tenant_id` equal to the platform tenant.
- The setup creates the first account with the right and no membership.
- A migration moves each `platform_admin` membership to the account right and removes the membership. A user-facing list shows the platform tenant in no case.

### The session

- A session is valid without a current organization. A Platform Admin with zero memberships signs in and reaches the platform screens.
- An account with zero memberships and no right also signs in. The user interface shows the empty state "no organizations, ask an administrator". This supersedes the AC-AU-06 rule "zero memberships give `invalid_credentials`".
- A session with no organization stores the internal platform tenant id in `sessions.tenant_id`, so the foreign key stays. The control plane never shows that tenant.
- An account with one or more memberships signs in to the chosen organization (ADR-0025). The session tenant is a real organization.

### The authorization

- A platform operation needs the account right. The control plane checks the right, not the tenant of the session. This supersedes the ADR-0025 rule that a platform operation needs the platform tenant membership.
- The platform operations run in the platform scope (`restichub.scope = 'platform'`, `PlatformTx`). They never run in a tenant transaction of the platform tenant.
- The right alone gives no access to tenant data: hosts, repositories, jobs, secrets and the tenant audit log. To work inside an organization, the Platform Admin adds a membership for itself through the platform users screen. The tenant role of that membership decides the access there (ADR-0006, ADR-0025).
- A Platform Admin that is a member of organization A uses the role of that membership in A. The right does not raise the role.

### The protections

- The last active Platform Admin cannot lose the right. The operation gives HTTP 409.
- A Platform Admin cannot remove the right from itself. The operation gives HTTP 403.
- A tenant-scope write on an account that holds the right is refused (HTTP 403). The Admin of an organization must not reset the password or the display name of such an account. It must not disable the account, and it must not change, disable or remove its membership. Only the platform scope manages such an account. The guard lives in the administrator database functions and in a trigger on `memberships`. The API repeats it.
- A grant of the right to an existing account requires a new password that only the Platform Admin chooses. The grant sets the password hash, sets the right and ends every session of the account in one transaction. A "must change at the next sign-in" flag is not enough. The Admin of an organization knows the old password and signs in before the first change. The account create sets the right with the password of the create, so it needs no extra password.
- Only a `SECURITY DEFINER` function owned by `restichub_auth` writes the right. The function sets the search path and a fail-closed scope check, and it takes the account row lock. The runtime role gets `EXECUTE` only. The rules of migrations 0016 to 0019 apply.

### The platform tenant row

- The platform tenant row stays. The `ca_keys` table and the signing keys reference it, and the platform audit records carry its id.
- The platform tenant never appears in the organization switcher, the organization list, the tenant API, a membership target or any other user-facing list.
- The platform tenant is never suspended or deleted.

### The self-add audit

- A membership that the Platform Admin adds for itself is an audit record in the platform audit and in the target organization's audit.
- A membership change through the platform routes is an audit record in the target organization. An account change is an audit record in the platform audit.

## Consequences

- An account has the Platform Admin right independently of its organizations.
- The switcher lists real organizations only. An account with no membership sees an empty state.
- The platform screens (Organizations, Platform users, Platform audit) open for a Platform Admin in any current organization. No organization switch is necessary.
- The platform right gives no tenant data access. The operator adds a membership to work in an organization.
- The platform tenant row is internal. A user-facing list never shows it.
- The sign-in of an account with zero memberships succeeds and shows the empty state. The answer no longer hides the absence of a membership.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep the platform tenant membership | The operator rejected it. The switcher shows the platform as an organization. |
| A second platform tenant for the right | The right belongs to the account, not to a tenant. |
| Remove the platform tenant row | The `ca_keys` table, the signing keys and the platform audit records reference it by a foreign key. |
| A nullable `sessions.tenant_id` | The foreign key and the Row-Level Security of the session table need a tenant. |

## Platform API tokens

- The MVP holds no platform-scope API token. A token cannot carry the Platform Admin right.
- The migration revokes every API token of the platform tenant with the role `platform_admin`. It keeps the rows for the audit trail.
- The control plane refuses to create a token with the role `platform_admin` and refuses to authenticate a token of the platform tenant. No token path gives `platform_admin = true`.
- A later record may add a platform token that is bound to its creator. It must state how the right of the creator binds to the token.
