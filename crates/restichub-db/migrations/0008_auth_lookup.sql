-- Cross-tenant authentication lookups (ADR-0023, section Local authentication; D5).
--
-- Sign-in and token authentication happen before the control plane knows the tenant. The runtime
-- role therefore cannot use the tenant policies. Three narrow `SECURITY DEFINER` functions do the
-- lookup.
--
-- The functions are owned by the group role `restichub_auth`. That role has `BYPASSRLS`, because
-- `FORCE ROW LEVEL SECURITY` also applies to the table owner. A database administrator creates
-- `restichub_auth` with `crates/restichub-db/sql/bootstrap-roles.sql` before the first migration.
--
-- Each function returns the named columns only. Each function sets the search path to `pg_catalog`
-- and `pg_temp`. Every relation in a function is schema-qualified. The runtime role `restichub_app`
-- gets `EXECUTE` and nothing else.
--
-- The caller checks the session expiry, the token expiry and revocation, and the `disabled` flag of
-- the user. The rate limit of sign-in belongs to task #18.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
    IF EXISTS (
        SELECT 1 FROM pg_roles
        WHERE rolname = 'restichub_auth'
          AND (rolcanlogin OR rolsuper OR NOT rolbypassrls)
    ) THEN
        RAISE EXCEPTION
            'the role restichub_auth must be NOLOGIN, NOSUPERUSER and BYPASSRLS; recreate it with crates/restichub-db/sql/bootstrap-roles.sql';
    END IF;
END
$$;

-- The definer role reads the lookup tables and creates the functions in the schema.
GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;
GRANT SELECT ON tenants, sessions, api_tokens TO restichub_auth;

CREATE FUNCTION auth_find_tenant(tenant_name text)
RETURNS TABLE (id uuid, status text)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT tenant.id, tenant.status
    FROM public.tenants AS tenant
    WHERE lower(tenant.name) = lower(tenant_name)
    LIMIT 1
$$;

CREATE FUNCTION auth_find_session(token bytea)
RETURNS TABLE (
    tenant_id uuid,
    session_id uuid,
    user_id uuid,
    expires_at timestamptz,
    last_seen_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT session.tenant_id, session.id, session.user_id, session.expires_at, session.last_seen_at
    FROM public.sessions AS session
    WHERE session.token_hash = token
    LIMIT 1
$$;

CREATE FUNCTION auth_find_api_token(token bytea)
RETURNS TABLE (
    tenant_id uuid,
    token_id uuid,
    role text,
    expires_at timestamptz,
    revoked_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT api_token.tenant_id, api_token.id, api_token.role, api_token.expires_at, api_token.revoked_at
    FROM public.api_tokens AS api_token
    WHERE api_token.token_hash = token
    LIMIT 1
$$;

ALTER FUNCTION auth_find_tenant(text) OWNER TO restichub_auth;
ALTER FUNCTION auth_find_session(bytea) OWNER TO restichub_auth;
ALTER FUNCTION auth_find_api_token(bytea) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION auth_find_tenant(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_session(bytea) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_api_token(bytea) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION auth_find_tenant(text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_session(bytea) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_api_token(bytea) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
