# ADR-0023: Control plane core: storage, authentication and secret format

- Status: partly superseded by ADR-0025 (installation-wide user accounts)
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

> Superseded by ADR-0025 for the account identity and the sign-in rate-limit keys.

- The password hash is Argon2id (`argon2` crate, m = 19456 KiB, t = 2, p = 1, PHC string).
- The minimum password length is 12.
- Sign-in has a rate limit for each user and each source IP.
- An unknown user and a wrong password give the same error and a similar time.

### Sessions

> Superseded by ADR-0025 for the scope of a password change: the change is installation-wide and it ends the sessions in all tenants.

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
- A platform operation that writes audit records uses `PlatformTx`. The method `enter_tenant` sets the tenant id in the same transaction and accepts only a tenant that the transaction created. A tenant creation, its key row, its first user and its audit records then commit together or not at all. The repository function `tenants::create_with_bootstrap` does this sequence. The function `audit::insert_platform_for` writes the audit record of an existing tenant (suspend, resume or delete). It sets the separate GUC `restichub.audit_tenant_id` for one insert and clears it. Only the policy `audit_log_platform_insert` reads the GUC, so the transaction keeps no tenant scope after the write.
- `FORCE ROW LEVEL SECURITY` also applies to the table owner. The authentication lookups `auth_find_tenant`, `auth_find_session` and `auth_find_api_token` are `SECURITY DEFINER` and owned by `restichub_auth`, a role with `BYPASSRLS`. Each function returns the named columns only and sets the search path. The runtime role gets `EXECUTE` and nothing else. The caller checks the session expiry, the token expiry and revocation, and the user `disabled` flag. The sign-in rate limit belongs to task #18.
- The runtime role has no `DELETE` on `tenant_keys`. A tenant has one active key version.

These notes also refine section Secrets. The record fixes the format of the encrypted values once.

- A KEK is 32 bytes from the operating system random number generator. A KEK has a key ID. The key ID is the first 16 hexadecimal characters of the SHA-256 digest of the bytes `restichub-kek-id-v1` followed by the key bytes. The key ID identifies the KEK during a rotation.
- The KEK file is a regular file. The control plane refuses a symbolic link and a file with the group or the other permission bits set. The condition is `mode & 0o077 == 0` on Unix. The control plane opens the file one time with `O_NOFOLLOW` and reads from the same handle.
- A wrapped DEK uses AES-256-GCM with a random 96-bit nonce. The associated data is the bytes `restichub-dek-v1\0`, then the 16 tenant ID bytes, then the 4 key version bytes in big-endian order.
- A secret value uses AES-256-GCM with a random 96-bit nonce. The associated data is the bytes `restichub-secret-v1\0`. Then come the 16 tenant ID bytes and the 16 secret ID bytes. Then come the 4 length bytes of the purpose in big-endian order and the purpose bytes.
- The `kek_check` value uses AES-256-GCM with a fixed plaintext. The associated data is the bytes `restichub-kek-check-v1\0`.
- The purpose of a secret value is one of `repository_password`, `s3_access_key`, `s3_secret_key` and `rest_server_password`. These strings are stable. A change makes every stored value of that purpose unreadable.
- The control plane MUST NOT start a child process that inherits the environment while the KEK is in `RESTICHUB_KEK`. On Unix, another process of the same user can read the environment through `/proc`.
- The write-only secret API creates, replaces, lists and removes the secrets of one tenant. The metadata read needs the Viewer role. The write needs the Admin role. The operations are tenant-only: the platform tenant gets HTTP 403. No endpoint returns a value. The read endpoints show the metadata and the marker `set`. The control plane deserialises a request value into a `SecretString`, so the value is zeroized when the type drops. The bytes that `hyper` buffers for a request body are not zeroized when the extractor drops them. The control plane accepts this residue as a residual risk. The buffer lives in the memory of the control plane process only. The control plane writes it to no file, no log and no argument.
- The delivery path reads the ciphertext of a secret and writes the `secret_delivered` audit record in one transaction. The caller must commit before it sends the value to the agent. A rollback removes the record, and the agent then does not receive the value. The control plane decrypts a secret with the key version that the row stores. A rotated tenant can then still deliver a secret that an older key version encrypted.

These notes refine section Local authentication. Issue #18 puts them into the code.

> Superseded by ADR-0025 for the rate-limit keys and the user administration rules.

- The platform tenant holds the role `platform_admin` only (FR-TN-02). The roles Admin, Operator and Viewer apply in one tenant. Migration 0011 adds a check constraint to `memberships` and `api_tokens` for this rule. The API layer refuses the role too.
- An actor cannot change, disable, delete or reset the password of a user whose role rank is above its own. An actor cannot grant a role above its own either.
- The sign-in rate limiter is in memory and per process. A second control plane instance has its own buckets. This is acceptable while one instance runs (ADR-0018). A shared limiter arrives with high availability.
- Each capped limiter map has a hard cap of 100 000 buckets. When a map is full, the limiter evicts the oldest buckets rather than grow. An inline sweep runs at most once per second, and a periodic sweep in the control plane removes the idle buckets. An IPv6 source is keyed by the /64 network.
- The resolved `(tenant_id, user_id)` identities live in a separate uncapped map; its size is bounded by the number of real users.
- The limiter keys the user bucket by the resolved `(tenant_id, user_id)` when the user is known. It keys the bucket by the normalized `tenant + username` otherwise.
- A password hash never holds a database transaction. A handler reads the user, ends the transaction, verifies or hashes, and then opens a second transaction for the write.
- A global semaphore bounds the number of concurrent Argon2id hashes to `auth.max_concurrent_hashes` (the number of CPUs by default). When the semaphore is saturated for two seconds, the request gets HTTP 503 `busy` with `Retry-After`.
- The failed sign-in audit record never holds the attempted username.
- For a known user the record holds the user identifier and the system actor.
- For an unknown user the record holds the marker `user_known` set to false only.
- A password typed into the username field cannot reach the append-only log.
- The setup endpoint takes the CSRF header and uses the per-IP rate limit of sign-in. The platform tenant lock and the `FOR UPDATE` row lock of the setup token serialize two setup requests with the same token.
- The one-time setup token is the only secret that the control plane ever logs.
- The server generates it at start when the platform tenant has no user.
- It stores only the SHA-256 hash and an expiry of 24 hours. It prints the value once at WARN level.
- The alternative is a default password. Every installation would share it and no operator would change it.
- The value expires and a restart generates a new one.
- The token lives in the table `setup_tokens` (migration 0010). The server marks it used and never removes it.
- The control plane maintenance task removes the sessions past the absolute lifetime and the expired and used setup tokens. It is not job scheduling (ADR-0002).
- An API token is owned by the tenant, not by the user that created it. A token stays valid while the tenant is active, also when the creator is disabled. The endpoint `DELETE /api/v1/users/{id}` disables the user and removes its sessions, and it leaves the tokens in place.
- For local development over plain HTTP, the setting `server.insecure_cookies` drops the `Secure` attribute and the `__Host-` prefix of the session cookie.
- A plain HTTP network can then read the session.
- Another host on the same parent domain can set a cookie with the same name. This is cookie tossing.
- The server logs a loud warning when the setting is on. The default is off.

These notes refine sections Tenant isolation and Secrets for the tenant administration (issue #20).

- The platform endpoints manage the tenants: create, list, read, suspend, resume and delete. The operations are `platform.tenants.read` and `platform.tenants.write`. The minimum role is Platform Admin. A tenant scope gets HTTP 403. The endpoints return the count of the users only. They return no user list and no secret of another tenant.
- The list returns the status, the creation time and the user count of each tenant. It filters by status and it pages with a keyset cursor. The default list excludes a deleted tenant. The count comes from the function `tenant_user_counts`, which is `SECURITY DEFINER`. The function returns rows only when the scope is `platform`; a tenant transaction gets no row.
- The create validates the name: 1 to 63 characters from `a-z`, `0-9` and `-`, the first character a letter, and not `platform`. The index `tenants_name_lower_unique` makes the name unique and gives HTTP 409 on a duplicate.
- The create writes the tenant row, the first Admin user and its membership in one transaction. The transaction also writes a fresh data encryption key (DEK). The current key encryption key (KEK) wraps the DEK. The record `tenant_create` belongs to the platform tenant. The record `user_create` belongs to the new tenant. Both records hold the actor and the source IP of the request.
- A status change names the allowed source status in the update (suspend: `active`; resume: `suspended`; delete: `suspended`). A concurrent change therefore wins or loses cleanly. When the update touches no row, the function re-reads the status. A status equal to the target is an idempotent no-op (HTTP 204, no record). Another status is a conflict (HTTP 409).
- The suspend, the resume and the delete write the audit record in the platform tenant and in the affected tenant. The status change and the records commit in one transaction. The delete is a soft delete: the status becomes `deleted`, the sign-in and the API tokens stop, and no row is removed in the MVP. A later retention task removes the rows of a deleted tenant. The platform tenant is never suspended or deleted (HTTP 409).
- The function `tenants::is_dispatch_allowed` returns true for an `active` tenant only. The dispatcher of epic #7 uses it. The agents of a suspended tenant stay connected (epic #1).
- The server loads the KEK at start from `secrets.kek_file` or from the environment variable `RESTICHUB_KEK`. It verifies the `kek_check` row of the platform tenant, or creates the row on the first start. It takes an advisory lock before the read, so two instances create the row once. It refuses to start with an incorrect KEK and shows the cause (AC-RP-02).


