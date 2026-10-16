-- The password reset of an account by the Platform Admin (ADR-0025, ADR-0029).
--
-- `auth_admin_set_password` needs a membership in the current tenant, so the platform scope cannot
-- use it for an account of any organization. This function needs the platform tenant or the
-- platform scope and no membership. The caller ends the sessions of the account in the same
-- transaction. The status is `ok`, `not_found` or `forbidden`.

GRANT CREATE ON SCHEMA public TO restichub_auth;

CREATE FUNCTION auth_platform_set_password(account_id uuid, new_password_hash text)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    current_tenant uuid := NULLIF(current_setting('restichub.tenant_id', true), '')::uuid;
BEGIN
    IF current_tenant IS DISTINCT FROM '00000000-0000-7000-8000-000000000000'::uuid
       AND current_setting('restichub.scope', true) IS DISTINCT FROM 'platform'
    THEN
        RETURN 'forbidden';
    END IF;
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    UPDATE public.users
    SET password_hash = new_password_hash, updated_at = now()
    WHERE id = account_id;
    RETURN 'ok';
END
$$;

ALTER FUNCTION auth_platform_set_password(uuid, text) OWNER TO restichub_auth;
REVOKE ALL ON FUNCTION auth_platform_set_password(uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION auth_platform_set_password(uuid, text) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
