-- Grants for the runtime role (ADR-0023, sections Roles and Tenant isolation).
--
-- The role `restichub_app` is an installation prerequisite. A database administrator creates it
-- with `crates/restichub-db/sql/bootstrap-roles.sql` before the first migration. The role is not
-- the table owner and has no `BYPASSRLS`.
--
-- One ResticHub installation per PostgreSQL cluster: the group role `restichub_app` is shared by
-- every database of one installation. Do not put two installations in one cluster with this role
-- model (M5).
--
-- The migration runs as the owner login. The control plane runtime pool starts every transaction
-- with `SET LOCAL ROLE restichub_app`. The runtime login is a member of `restichub_app`.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_app') THEN
        RAISE EXCEPTION
            'the role restichub_app is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_roles
        WHERE rolname = 'restichub_app'
          AND (rolsuper OR rolbypassrls OR rolcreaterole OR rolcreatedb)
    ) THEN
        RAISE EXCEPTION
            'the role restichub_app has a forbidden attribute (SUPERUSER, BYPASSRLS, CREATEROLE or CREATEDB); recreate it with crates/restichub-db/sql/bootstrap-roles.sql';
    END IF;
END
$$;

-- The runtime role must not create objects in the schema (L3).
REVOKE CREATE ON SCHEMA public FROM PUBLIC;

GRANT USAGE ON SCHEMA public TO restichub_app;

-- The runtime role has full data manipulation on the tenant tables. Row-Level Security limits
-- every query.
GRANT SELECT, INSERT, UPDATE, DELETE ON
    tenants,
    users,
    memberships,
    sessions,
    api_tokens,
    tenant_keys,
    kek_check,
    secrets
TO restichub_app;

-- A key is rotated, never removed (D4).
REVOKE DELETE ON tenant_keys FROM restichub_app;

-- The audit log is append-only. The runtime role has SELECT and INSERT only.
GRANT SELECT, INSERT ON audit_log TO restichub_app;

-- The control plane compares the applied migration version at start (N4).
GRANT SELECT ON _sqlx_migrations TO restichub_app;
