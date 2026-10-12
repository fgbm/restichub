-- The per-tenant user count for the platform administration (issue #20, FR-TN-02).
--
-- The Platform Admin lists the tenants with the number of users of each tenant. The platform scope
-- cannot read the `users` table (migration 0006), and the endpoint must return the count only, not
-- a user list. One narrow `SECURITY DEFINER` function returns the count of each tenant. It is owned
-- by `restichub_auth`, which has `BYPASSRLS`, and it returns no user row.
--
-- The function returns rows only when the scope is `platform`. A tenant transaction gets no row, so
-- the count is not a second read path into the `users` table.
--
-- The function follows the rules of migration 0008: it sets the search path, it schema-qualifies
-- every relation, and the runtime role `restichub_app` gets `EXECUTE` and nothing else.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

-- The definer role reads the `users` table and creates the function in the schema.
GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;
GRANT SELECT ON users TO restichub_auth;

CREATE FUNCTION tenant_user_counts()
RETURNS TABLE (tenant_id uuid, user_count bigint)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT member.tenant_id, count(*)::bigint
    FROM public.users AS member
    WHERE current_setting('restichub.scope', true) = 'platform'
    GROUP BY member.tenant_id
$$;

ALTER FUNCTION tenant_user_counts() OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION tenant_user_counts() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION tenant_user_counts() TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
