-- The Platform Admin right on the installation account (ADR-0029, issue #61).
--
-- The Platform Admin right becomes a boolean on the account row. The existing `platform_admin`
-- memberships move to the right and the memberships are removed. The membership roles are Admin,
-- Operator and Viewer only. The platform tenant stays internal: the `ca_keys` table, the signing
-- keys and the platform audit records reference it, so the row stays, but no membership points to
-- it and no user-facing list shows it.
--
-- The platform API token keeps the role `platform_admin` in the platform tenant only. The creator
-- of a platform token holds no membership in the platform tenant, so the creator-membership
-- trigger skips the platform tenant.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;

-- The owner moves the memberships across tenants, so the force is off for the move (as in
-- migration 0016).
ALTER TABLE users NO FORCE ROW LEVEL SECURITY;
ALTER TABLE memberships NO FORCE ROW LEVEL SECURITY;

ALTER TABLE users ADD COLUMN platform_admin boolean NOT NULL DEFAULT false;

-- Each `platform_admin` membership becomes the right of its account.
UPDATE users
SET platform_admin = true
WHERE id IN (SELECT user_id FROM memberships WHERE role = 'platform_admin');

DELETE FROM memberships WHERE role = 'platform_admin';

-- The membership role set drops `platform_admin`, and the platform tenant is never a target.
ALTER TABLE memberships DROP CONSTRAINT memberships_role_check;
ALTER TABLE memberships DROP CONSTRAINT memberships_platform_admin_only_platform;
ALTER TABLE memberships DROP CONSTRAINT memberships_platform_tenant_role;
ALTER TABLE memberships
    ADD CONSTRAINT memberships_role_check
        CHECK (role IN ('admin', 'operator', 'viewer'));
ALTER TABLE memberships
    ADD CONSTRAINT memberships_not_platform_tenant
        CHECK (tenant_id <> '00000000-0000-7000-8000-000000000000');

ALTER TABLE users FORCE ROW LEVEL SECURITY;
ALTER TABLE memberships FORCE ROW LEVEL SECURITY;

-- The active administrator count of a tenant counts the role `admin` only (ADR-0029).
CREATE OR REPLACE FUNCTION tenant_admin_counts()
RETURNS TABLE (tenant_id uuid, admin_count bigint)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT member.tenant_id, count(*)::bigint
    FROM public.memberships AS member
    JOIN public.users AS account ON account.id = member.user_id
    WHERE current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
      AND member.role = 'admin'
      AND NOT member.disabled
      AND NOT account.disabled
    GROUP BY member.tenant_id
$$;

-- The creator-membership trigger skips the platform tenant: a platform API token has no membership
-- in its tenant (ADR-0029).
CREATE OR REPLACE FUNCTION check_creator_membership() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.created_by IS NOT NULL
       AND NEW.tenant_id <> '00000000-0000-7000-8000-000000000000'::uuid
       AND NOT EXISTS (
           SELECT 1 FROM public.memberships AS member
           WHERE member.user_id = NEW.created_by
             AND member.tenant_id = NEW.tenant_id
       )
    THEN
        RAISE EXCEPTION
            'the creator % holds no membership in the tenant % of the row',
            NEW.created_by,
            NEW.tenant_id
            USING ERRCODE = '23503';
    END IF;
    RETURN NEW;
END
$$;

-- The account lookups also return the Platform Admin right.
DROP FUNCTION auth_find_account(text);

CREATE FUNCTION auth_find_account(account_name text)
RETURNS TABLE (
    id uuid,
    username text,
    password_hash text,
    display_name text,
    disabled boolean,
    last_used_tenant_id uuid,
    platform_admin boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.password_hash, account.display_name,
           account.disabled, account.last_used_tenant_id, account.platform_admin
    FROM public.users AS account
    WHERE lower(account.username) = lower(account_name)
    LIMIT 1
$$;

DROP FUNCTION auth_find_account_by_id(uuid);

CREATE FUNCTION auth_find_account_by_id(account_id uuid)
RETURNS TABLE (
    id uuid,
    username text,
    display_name text,
    disabled boolean,
    last_used_tenant_id uuid,
    platform_admin boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.display_name, account.disabled,
           account.last_used_tenant_id, account.platform_admin
    FROM public.users AS account
    WHERE account.id = account_id
    LIMIT 1
$$;

-- The platform account list also returns the right.
DROP FUNCTION auth_platform_accounts();

CREATE FUNCTION auth_platform_accounts()
RETURNS TABLE (
    user_id uuid,
    username text,
    display_name text,
    email text,
    email_verified boolean,
    disabled boolean,
    platform_admin boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.display_name, account.email,
           account.email_verified, account.disabled, account.platform_admin
    FROM public.users AS account
    WHERE current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
    ORDER BY account.username
$$;

-- The platform account create takes the right as a parameter.
DROP FUNCTION auth_platform_create_account(uuid, text, text, text, text);

CREATE FUNCTION auth_platform_create_account(
    new_id uuid,
    account_name text,
    new_password_hash text,
    new_display_name text,
    new_email text,
    new_platform_admin boolean
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
    INSERT INTO public.users
        (id, tenant_id, username, password_hash, display_name, email, platform_admin)
    VALUES (
        new_id,
        '00000000-0000-7000-8000-000000000000'::uuid,
        account_name,
        new_password_hash,
        new_display_name,
        new_email,
        new_platform_admin
    );
    PERFORM 1 FROM public.users WHERE id = new_id FOR UPDATE;
    RETURN 'ok';
END
$$;

-- Creates the first Platform Admin account with the right and no membership. The setup uses it in
-- the platform tenant; the platform scope is accepted too. `false` when the name exists or the
-- caller is outside the scope.
CREATE FUNCTION auth_create_platform_admin(
    new_id uuid,
    account_name text,
    new_password_hash text,
    new_display_name text
)
RETURNS boolean
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
        RETURN false;
    END IF;
    IF EXISTS (SELECT 1 FROM public.users WHERE lower(username) = lower(account_name)) THEN
        RETURN false;
    END IF;
    INSERT INTO public.users
        (id, tenant_id, username, password_hash, display_name, platform_admin)
    VALUES (
        new_id,
        '00000000-0000-7000-8000-000000000000'::uuid,
        account_name,
        new_password_hash,
        new_display_name,
        true
    );
    PERFORM 1 FROM public.users WHERE id = new_id FOR UPDATE;
    RETURN true;
END
$$;

-- The count of the active Platform Admins. It needs the platform tenant or the platform scope; it
-- returns 0 otherwise.
CREATE FUNCTION auth_platform_admin_count()
RETURNS bigint
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT count(*)::bigint
    FROM public.users AS account
    WHERE account.platform_admin
      AND NOT account.disabled
      AND (
          current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
          OR NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
             = '00000000-0000-7000-8000-000000000000'::uuid
      )
$$;

-- True when the account holds the active right. It needs the platform tenant or the platform scope.
CREATE FUNCTION auth_is_platform_admin(account_id uuid)
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT (
        current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
        OR NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
           = '00000000-0000-7000-8000-000000000000'::uuid
    )
    AND EXISTS (
        SELECT 1 FROM public.users AS account
        WHERE account.id = account_id AND account.platform_admin AND NOT account.disabled
    )
$$;

-- Writes the right of an account. The status is `ok`, `not_found` or `forbidden`. The function
-- needs the platform tenant or the platform scope and takes the account row lock.
CREATE FUNCTION auth_set_platform_admin(account_id uuid, new_value boolean)
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
    SET platform_admin = new_value, updated_at = now()
    WHERE id = account_id;
    RETURN 'ok';
END
$$;

ALTER FUNCTION tenant_admin_counts() OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account(text) OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account_by_id(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_accounts() OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_create_account(uuid, text, text, text, text, boolean) OWNER TO restichub_auth;
ALTER FUNCTION auth_create_platform_admin(uuid, text, text, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_admin_count() OWNER TO restichub_auth;
ALTER FUNCTION auth_is_platform_admin(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_platform_admin(uuid, boolean) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION tenant_admin_counts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account_by_id(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_accounts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_create_account(uuid, text, text, text, text, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_create_platform_admin(uuid, text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_admin_count() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_is_platform_admin(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_platform_admin(uuid, boolean) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION tenant_admin_counts() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_account(text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_account_by_id(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_accounts() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_create_account(uuid, text, text, text, text, boolean) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_create_platform_admin(uuid, text, text, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_platform_admin_count() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_is_platform_admin(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_set_platform_admin(uuid, boolean) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
