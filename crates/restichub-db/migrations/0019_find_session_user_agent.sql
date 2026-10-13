-- `auth_find_session` also returns the user agent of the session (ADR-0025).
--
-- The tenant switch keeps the user agent of the source session on the new session. The lookup
-- function therefore returns the column. The return type changes, so the function is dropped and
-- recreated with its owner and grants. The definer role needs CREATE on the schema while the new
-- owner is set, as in migration 0008.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;

DROP FUNCTION auth_find_session(bytea);

CREATE FUNCTION auth_find_session(token bytea)
RETURNS TABLE (
    tenant_id uuid,
    session_id uuid,
    user_id uuid,
    expires_at timestamptz,
    last_seen_at timestamptz,
    user_agent text
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT session.tenant_id, session.id, session.user_id, session.expires_at, session.last_seen_at,
           session.user_agent
    FROM public.sessions AS session
    WHERE session.token_hash = token
    LIMIT 1
$$;

ALTER FUNCTION auth_find_session(bytea) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION auth_find_session(bytea) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION auth_find_session(bytea) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
