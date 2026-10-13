-- Platform user management and the account email (ADR-0028, issue #60).
--
-- The account gains the optional email and the verified flag. The platform administration creates
-- an installation account, lists every account with its memberships, changes the role of a
-- membership and removes it. Every write runs through a `SECURITY DEFINER` function owned by
-- `restichub_auth`, as migrations 0016 to 0019 do: each function sets the search path, checks its
-- scope at the database level, and the runtime role gets `EXECUTE` and nothing else.
--
-- The account email is unique in the installation when it is set, without the letter case
-- (ADR-0028). The MVP writes `email_verified` as false only.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;

ALTER TABLE users
    ADD COLUMN email text,
    ADD COLUMN email_verified boolean NOT NULL DEFAULT false;

CREATE UNIQUE INDEX users_email_lower_unique ON users (lower(email)) WHERE email IS NOT NULL;

-- The definer role removes a membership and reads the account email.
GRANT DELETE ON memberships TO restichub_auth;

-- The count of the active administrators of each tenant. The platform list of the organizations
-- uses it to compute the derived admin state. A tenant transaction gets no row.
CREATE FUNCTION tenant_admin_counts()
RETURNS TABLE (tenant_id uuid, admin_count bigint)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT member.tenant_id, count(*)::bigint
    FROM public.memberships AS member
    JOIN public.users AS account ON account.id = member.user_id
    WHERE current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
      AND member.role IN ('admin', 'platform_admin')
      AND NOT member.disabled
      AND NOT account.disabled
    GROUP BY member.tenant_id
$$;

-- Lists every account of the installation for the platform administration. The function returns a
-- row only in the platform scope.
CREATE FUNCTION auth_platform_accounts()
RETURNS TABLE (
    user_id uuid,
    username text,
    display_name text,
    email text,
    email_verified boolean,
    disabled boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.display_name, account.email,
           account.email_verified, account.disabled
    FROM public.users AS account
    WHERE current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
    ORDER BY account.username
$$;

-- Lists every membership of every account for the platform administration. The function returns a
-- row only in the platform scope.
CREATE FUNCTION auth_platform_memberships()
RETURNS TABLE (
    user_id uuid,
    tenant_id uuid,
    tenant_name text,
    role text,
    disabled boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT member.user_id, member.tenant_id, tenant.name, member.role, member.disabled
    FROM public.memberships AS member
    JOIN public.tenants AS tenant ON tenant.id = member.tenant_id
    WHERE current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
    ORDER BY tenant.name
$$;

-- Creates an installation account (Platform Admin only). The status is `ok`, `username_taken`,
-- `email_taken` or `forbidden`. The function requires the platform scope, takes the account row
-- lock, and refuses a duplicate username or email.
CREATE FUNCTION auth_platform_create_account(
    new_id uuid,
    account_name text,
    new_password_hash text,
    new_display_name text,
    new_email text
)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF current_setting('restichub.scope', true) IS DISTINCT FROM 'platform' THEN
        RETURN 'forbidden';
    END IF;
    IF EXISTS (SELECT 1 FROM public.users WHERE lower(username) = lower(account_name)) THEN
        RETURN 'username_taken';
    END IF;
    IF new_email IS NOT NULL
       AND EXISTS (SELECT 1 FROM public.users WHERE lower(email) = lower(new_email)) THEN
        RETURN 'email_taken';
    END IF;
    INSERT INTO public.users (id, tenant_id, username, password_hash, display_name, email)
    VALUES (
        new_id,
        '00000000-0000-7000-8000-000000000000'::uuid,
        account_name,
        new_password_hash,
        new_display_name,
        new_email
    );
    PERFORM 1 FROM public.users WHERE id = new_id FOR UPDATE;
    RETURN 'ok';
END
$$;

-- Changes the role of a membership (Platform Admin only). The status is `ok`, `not_found` or
-- `forbidden`. The function requires the platform scope, refuses the platform tenant and takes the
-- account row lock.
CREATE FUNCTION auth_platform_set_membership_role(
    account_id uuid,
    target_tenant_id uuid,
    new_role text
)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF current_setting('restichub.scope', true) IS DISTINCT FROM 'platform' THEN
        RETURN 'forbidden';
    END IF;
    IF target_tenant_id = '00000000-0000-7000-8000-000000000000'::uuid THEN
        RETURN 'forbidden';
    END IF;
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    UPDATE public.memberships
    SET role = new_role
    WHERE user_id = account_id AND tenant_id = target_tenant_id;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    RETURN 'ok';
END
$$;

-- Removes a membership (Platform Admin only). The status is `ok`, `not_found` or `forbidden`. The
-- function requires the platform scope, refuses the platform tenant and takes the account row lock.
CREATE FUNCTION auth_platform_remove_membership(account_id uuid, target_tenant_id uuid)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF current_setting('restichub.scope', true) IS DISTINCT FROM 'platform' THEN
        RETURN 'forbidden';
    END IF;
    IF target_tenant_id = '00000000-0000-7000-8000-000000000000'::uuid THEN
        RETURN 'forbidden';
    END IF;
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    DELETE FROM public.memberships
    WHERE user_id = account_id AND tenant_id = target_tenant_id;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    RETURN 'ok';
END
$$;

-- Sets the email of an account (Platform Admin only). The status is `ok`, `not_found` or
-- `forbidden`. The function needs the platform tenant or the platform scope. The unique index
-- refuses a duplicate email. The flag `email_verified` returns to false with a new email.
CREATE FUNCTION auth_platform_set_email(account_id uuid, new_email text)
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
    UPDATE public.users
    SET email = new_email, email_verified = false, updated_at = now()
    WHERE id = account_id;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    RETURN 'ok';
END
$$;

ALTER FUNCTION tenant_admin_counts() OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_accounts() OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_memberships() OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_create_account(uuid, text, text, text, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_set_membership_role(uuid, uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_remove_membership(uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_set_email(uuid, text) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION tenant_admin_counts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_accounts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_memberships() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_create_account(uuid, text, text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_set_membership_role(uuid, uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_remove_membership(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_set_email(uuid, text) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION tenant_admin_counts() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_accounts() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_memberships() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_create_account(uuid, text, text, text, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_set_membership_role(uuid, uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_remove_membership(uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_set_email(uuid, text) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
