# ADR-0028: Organization creation without an administrator, and platform user management

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-TN-02, FR-AU-04

## Context

ADR-0023 creates an organization (a tenant) together with its first Admin in one transaction. The create endpoint `POST /api/v1/platform/tenants` requires the field `first_admin`. The platform scope holds no other way to add an account. The endpoint `POST /api/v1/platform/users/{id}/memberships` needs an existing account. The platform read of the accounts does not exist. The operator therefore creates an organization only with a user that nobody asked for.

The operator decided that the account management belongs to the platform. A person has one installation-wide account (ADR-0025). The Platform Admin creates an organization and then adds the people. Issue #60 asks for this change.

The plan for 1.0 uses invitations by email (see the comment of issue #60). An invitation is its own entity and it needs the email address of the account. This record prepares the account model for the invitations. It adds no invitation and no mail sending now.

## Decision

### Organization create

- We remove `first_admin` from `POST /api/v1/platform/tenants`. An organization is created by name only, together with its data encryption key row.
- The transaction writes the tenant row, the key row and the audit record `tenant_create`. It writes no user and no `user_create` record. This supersedes the ADR-0023 rule "The create writes the tenant row, the first Admin user and its membership in one transaction."
- The record `tenant_create` belongs to the platform tenant. It holds the actor and the source IP of the request.

### An organization without an administrator

- An organization without an active Admin has the derived state `no administrator`. The control plane computes the state from the memberships: the count of the active Admins is zero. The state is not a stored lifecycle status, so it cannot drift.
- The lifecycle status (active, suspended, deleted) stays separate. The tenant API exposes the derived state as the field `admin_state` with the values `none` and `present`.
- An active Admin is a membership with the role Admin or Platform Admin, where the membership is not disabled and the account is not disabled.

### The account email

- The account model gains the optional field `email` and the boolean field `email_verified` with the default false.
- The comparison of the email ignores the letter case. An email is unique in the installation when it is set: two accounts cannot hold the same email. A second account with the same email gives HTTP 409 `email_taken`.
- The endpoints `POST /api/v1/platform/users` and `PATCH /api/v1/platform/users/{id}` accept the optional email.
- The MVP treats no email as verified. The flag `email_verified` stays false. The invitations of 1.0 verify the address and set the flag.
- The account create and the account update write the email. No invitation and no mail sending belongs to this issue.

### Platform user management

- The Platform Admin creates an account: `POST /api/v1/platform/users` with the username, the display name, the password, the optional email and the optional memberships. A membership names an organization and a role.
- The username and the email are unique in the installation. A clash gives HTTP 409 `username_taken` or `email_taken`.
- The Platform Admin lists every account with its memberships: `GET /api/v1/platform/users`. Each item holds the account state and each membership with the organization, the role and the disabled flag. The operation is `PlatformUsersRead`. A tenant scope gets HTTP 403.
- The Platform Admin manages a membership: `POST /api/v1/platform/users/{id}/memberships` adds it, `PATCH /api/v1/platform/users/{id}/memberships/{tenant_id}` changes the role, and `DELETE /api/v1/platform/users/{id}/memberships/{tenant_id}` removes it.
- The platform routes refuse the platform tenant as a membership target: the setup creates the Platform Admin membership (ADR-0023, ADR-0025).
- The platform routes keep the last active Platform Admin of the platform tenant. An operation that would remove it gives HTTP 409.
- The tenant rules of ADR-0025 do not change. A tenant Admin cannot add a foreign account, cannot touch a shared account, and reads the accounts of its members only (AC-AU-07).

### Database

- Every write runs through a `SECURITY DEFINER` function owned by `restichub_auth`, as migrations 0016 to 0019 do. Each function sets the search path, the runtime role gets `EXECUTE` only, and the function checks its scope at the database level.
- The account create takes the account row lock. The functions use the platform scope.

## Consequences

- An organization exists without a user. The Platform Admin adds the people through the platform user management.
- The organization list shows a "no administrator" mark. The mark is always current, because the control plane computes it from the memberships.
- The account holds an email that the invitations of 1.0 use. The MVP treats the email as unverified.
- The create endpoint of an organization no longer needs a password hash, so the endpoint is faster.
- Two tenants cannot share a username or an email.
- A person that the Platform Admin creates with no membership cannot sign in: zero memberships give `invalid_credentials` (ADR-0025). The organization list shows the count of the users and the admin state, so the operator sees the empty organization.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep `first_admin` optional | The operator decided that the organization create holds no user. Two paths to create an account add a second code path and a second audit shape. |
| A stored lifecycle status `no administrator` | The status drifts from the memberships. A derived value is always correct. |
| No email in the MVP | The invitations of 1.0 need the email. A later schema change of the account is expensive (ADR-0025). |
| A required unique email | The setup and the existing accounts hold no email. A required field breaks them. |
