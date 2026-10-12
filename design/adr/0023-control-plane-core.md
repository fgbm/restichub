# ADR-0023: Control plane core: storage, authentication and secret format

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-TN-01..03, FR-TN-05, FR-RP-03, FR-RP-05, FR-AU-01, FR-AU-03

## Context

The MVP needs the control plane core. The core holds the database schema, the tenant isolation, the local users, the sessions, the API tokens, the encrypted secrets and the audit log. Later epics (repositories, profiles, scheduler, agent protocol) build on this core.

This record fixes the shape of the core once. Later work then does not change the schema or the protocol. The record also defines the secret format and the database isolation, because a change after the first migration is expensive (ADR-0006, ADR-0007).

## Decision

### Crates

- We create three crates: `restichub-db` (connection pool, migrations, tenant-scoped transactions), `restichub-secrets` (KEK provider, envelope encryption) and `restichub-cp` (binary, axum API, authentication, authorization, audit service). Each crate has a narrow public API (ADR-0007).

### Database

- The database is PostgreSQL 16 or later. We use `sqlx` 0.8 with tokio and rustls.
- We use runtime query functions (`sqlx::query_as` with explicit types). We do not use the compile-time query macros, so the build needs no database.
- Migrations live in `crates/restichub-db/migrations/`. The control plane embeds them with `sqlx::migrate!` and applies them at start.
- IDs are UUIDv7. Times are `timestamptz` in UTC.

### Tenant isolation

- Each table has `tenant_id uuid NOT NULL` (AGENTS.md rule).
- A reserved platform tenant (a fixed UUID and one row in `tenants`) holds the Platform Admin users and the platform audit records.
- The tenant tables use `ENABLE` and `FORCE ROW LEVEL SECURITY` with the policy `tenant_id = current_setting('restichub.tenant_id', true)::uuid`.
- The migration owner role and the runtime role `restichub_app` are different roles. The runtime role is not the table owner and has no `BYPASSRLS`.
- Each request runs in a transaction that sets `SET LOCAL restichub.tenant_id`.
- Platform operations use a separate, explicit code path (`PlatformTx`) that sets `restichub.scope = 'platform'`. Only the `tenants` table and the platform audit read path accept this scope.
- A query outside a scoped transaction is a compile-time impossibility in the API layer: a handler receives only a `TenantTx` or a `PlatformTx`.
- Access to an object of another tenant returns HTTP 404 (AC-TN-01).

### Local authentication

- The password hash is Argon2id (`argon2` crate, m = 19456 KiB, t = 2, p = 1, PHC string).
- The minimum password length is 12.
- Sign-in has a rate limit for each user and each source IP.
- An unknown user and a wrong password give the same error and a similar time.

### Sessions

- A session token is 256 random bits from the OS CSPRNG. The database keeps only the SHA-256 hash of the token.
- The cookie is `__Host-restichub_session` with HttpOnly, Secure, SameSite=Strict and Path=/.
- The idle timeout is 30 minutes. The absolute lifetime is 12 hours. Both are configurable.
- Sign-out and a password change delete the sessions.
- For cookie authentication, each unsafe method needs the header `X-ResticHub-CSRF: 1`. We use exactly this header name.

### API tokens

- The token format is `rh_` plus 32 random bytes in base64url.
- The database keeps the SHA-256 hash and a short display prefix.
- Each token has one tenant, one role and an optional expiry.
- The control plane shows the value one time. Authentication uses the bearer scheme.

### Roles

- The roles are Platform Admin, Admin, Operator and Viewer (requirements section 2).
- One authorization table "operation → minimum role" lives in the code. Each handler declares its operation. A test checks that each OpenAPI operation has an entry (AC-AU-02, AC-TN-01).

### Errors

- An error is RFC 9457 `application/problem+json` with a stable `code` field in snake_case.
- The response contains no internal details. The control plane logs a correlation ID.

### Secrets

This section refines ADR-0017.

- The KEK is 32 random bytes, base64 encoded, from `kek_file` (a file with mode 0600, checked on Unix) or the environment variable `RESTICHUB_KEK`. If both are set, the start fails.
- Each KEK has a key ID. Each tenant has a data encryption key (DEK). The KEK wraps the DEK with AES-256-GCM.
- A secret value uses AES-256-GCM with the DEK, a random 96-bit nonce and the associated data `tenant_id || secret_id || purpose`.
- A `kek_check` row holds encrypted known plaintext. The control plane refuses to start with an incorrect KEK and shows the cause (AC-RP-02).
- Plaintext buffers use `zeroize`.
- No API returns a secret value. The API shows only `set` and `updated_at`.

### Audit log

- The audit log is one append-only table. The runtime role has INSERT and SELECT only. A trigger rejects UPDATE and DELETE.
- The fields are id, tenant_id, time, actor (user, API token, agent or system), source IP, action, object type and ID, and diff as JSON.
- A secret field in a diff shows only `"<redacted>"`.
- The client IP comes from the socket. The control plane uses `X-Forwarded-For` only from configured trusted proxies.

### Configuration

- Configuration is a TOML file plus `RESTICHUB_*` environment variables that override it.
- The database password and the KEK come from a file path or the environment. They never come from a command-line argument.

### Tests

- A test that needs PostgreSQL reads `RESTICHUB_TEST_DATABASE_URL`. Each test creates its own database.
- CI adds a `postgres:16` service to the Linux Rust job. The Windows job skips these tests with a clear message.

## Consequences

- The schema and the isolation model are fixed before the other epics. Later epics add tables that obey the same rules.
- The build runs without a database, because the queries are runtime strings.
- The database layer is a second barrier below the application checks (ADR-0006).
- The control plane cannot start without a correct KEK. It shows the cause.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Diesel or SeaORM | The design already uses `sqlx` for the queue (ADR-0010). |
| bcrypt or scrypt | Argon2id is the current recommendation and resists GPU attacks. |
| JWT sessions | A JWT cannot be revoked on the server before its expiry. |
| A separate platform schema without `tenant_id` | It violates the AGENTS.md rule that each table has `tenant_id`. |

## Implementation notes

These notes refine section Tenant isolation. The decision itself does not change. Issue #16 puts them into the schema.

- The migrations do not create roles. A database administrator runs `crates/restichub-db/sql/bootstrap-roles.sql` once for each PostgreSQL cluster. The script creates the group role `restichub_app`, the definer role `restichub_auth` and the two login roles without a password. The administrator sets each password with `\password`. One ResticHub installation uses one PostgreSQL cluster.
- The migrations run as a separate command, `restichub-cp migrate`. The command reads only the migration URL and password (`database_migration_url`), applies the migrations and stops. The server command `restichub-cp serve` accepts no migration credentials. The Docker Compose setup in epic #11 runs `restichub-cp migrate` as a one-shot service before the server.
- The runtime login owns no object and is a member of `restichub_app`. The owner login runs the migrations and owns the schema.
- The server compares the applied migration version in `_sqlx_migrations` with the latest embedded version at start. It refuses to start if the database is behind or ahead. The runtime role has `SELECT` on `_sqlx_migrations` only.
- The control plane checks the runtime role at start. It stops the start in these cases. The role `restichub_app` has SUPERUSER, BYPASSRLS, CREATEROLE or CREATEDB. The runtime login is a superuser or has BYPASSRLS. The runtime login can become such a role. The runtime login, or a role it can become, owns an object, a namespace, a function or the database. The runtime login, or a role it can become, is a member of `pg_database_owner`. The runtime login is not a member of `restichub_app`.
- A platform operation that writes audit records uses `PlatformTx`. The method `enter_tenant` sets the tenant id in the same transaction and accepts only a tenant that the transaction created. A tenant creation, its key row, its first user and its audit record then commit together or not at all. The repository function `tenants::create_with_bootstrap` does this sequence.
- `FORCE ROW LEVEL SECURITY` also applies to the table owner. The authentication lookups `auth_find_tenant`, `auth_find_session` and `auth_find_api_token` are `SECURITY DEFINER` and owned by `restichub_auth`, a role with `BYPASSRLS`. Each function returns the named columns only and sets the search path. The runtime role gets `EXECUTE` and nothing else. The caller checks the session expiry, the token expiry and revocation, and the user `disabled` flag. The sign-in rate limit belongs to task #18.
- The runtime role has no `DELETE` on `tenant_keys`. A tenant has one active key version.
