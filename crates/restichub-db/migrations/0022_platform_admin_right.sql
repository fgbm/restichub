-- The Platform Admin right on the installation account (ADR-0029, issue #61).
--
-- The Platform Admin right becomes a boolean on the account row. The existing `platform_admin`
-- memberships move to the right and the memberships are removed. The membership roles are Admin,
-- Operator and Viewer only. The platform tenant stays internal: the `ca_keys` table, the signing
-- keys and the platform audit records reference it, so the row stays, but no membership points to
-- it and no user-facing list shows it.
--
-- The MVP holds no platform-scope API token. Every API token of the platform tenant with the role
-- `platform_admin` is revoked (the rows stay); the code refuses to create or to authenticate one.
-- A later record may add a platform token bound to its creator.
--
-- A tenant-scope write on a Platform Admin account is refused: the tenant Admin of an organization
-- must not change an account that holds the installation-wide right. The guard lives in the
-- administrator functions and in a trigger on `memberships`.

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

-- Each enabled `platform_admin` membership becomes the right of its account. A disabled membership
-- is a revoked Platform Admin and gives no right.
UPDATE users
SET platform_admin = true
WHERE id IN (
    SELECT user_id FROM memberships WHERE role = 'platform_admin' AND NOT disabled
);

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

-- The MVP holds no platform-scope API token. Every platform token is revoked; the rows stay for the
-- audit trail (ADR-0029). The revoke runs after the creator-membership trigger skips the platform
-- tenant, so the update passes.
ALTER TABLE api_tokens NO FORCE ROW LEVEL SECURITY;

UPDATE api_tokens
SET revoked_at = now()
WHERE tenant_id = '00000000-0000-7000-8000-000000000000'::uuid
  AND role = 'platform_admin'
  AND revoked_at IS NULL;

ALTER TABLE api_tokens FORCE ROW LEVEL SECURITY;

-- A tenant-scope write to the membership of a Platform Admin is refused: the Admin of an
-- organization must not change the role, disable or remove the membership of an account that holds
-- the installation-wide right (ADR-0029). The platform scope and the platform tenant manage it.
-- The function runs as its owner, so it reads the right of the account.
CREATE FUNCTION check_platform_admin_membership() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    target uuid;
BEGIN
    IF current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
       OR NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
          = '00000000-0000-7000-8000-000000000000'::uuid
    THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    target := CASE WHEN TG_OP = 'DELETE' THEN OLD.user_id ELSE NEW.user_id END;
    IF EXISTS (SELECT 1 FROM public.users WHERE id = target AND platform_admin) THEN
        RAISE EXCEPTION
            'the organization scope may not change the membership of the Platform Admin %',
            target
            USING ERRCODE = '42501';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER memberships_platform_admin_guard
    BEFORE INSERT OR UPDATE OR DELETE ON memberships
    FOR EACH ROW EXECUTE FUNCTION check_platform_admin_membership();

-- An administrator write of the account password. It also refuses a Platform Admin from a tenant
-- scope (ADR-0029). The status is `ok`, `not_found`, `shared` or `forbidden`.
CREATE OR REPLACE FUNCTION auth_admin_set_password(account_id uuid, new_password_hash text)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    current_tenant uuid := NULLIF(current_setting('restichub.tenant_id', true), '')::uuid;
BEGIN
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.memberships
        WHERE user_id = account_id AND tenant_id = current_tenant
    ) THEN
        RETURN 'not_found';
    END IF;
    IF current_tenant <> '00000000-0000-7000-8000-000000000000'::uuid
       AND current_setting('restichub.scope', true) IS DISTINCT FROM 'platform'
       AND EXISTS (
           SELECT 1 FROM public.users WHERE id = account_id AND platform_admin
       ) THEN
        RETURN 'forbidden';
    END IF;
    IF current_tenant <> '00000000-0000-7000-8000-000000000000'::uuid
       AND current_setting('restichub.scope', true) IS DISTINCT FROM 'platform'
       AND EXISTS (
           SELECT 1
           FROM public.memberships AS member
           JOIN public.tenants AS tenant ON tenant.id = member.tenant_id
           WHERE member.user_id = account_id
             AND member.tenant_id <> current_tenant
             AND tenant.status <> 'deleted'
       ) THEN
        RETURN 'shared';
    END IF;
    UPDATE public.users
    SET password_hash = new_password_hash, updated_at = now()
    WHERE id = account_id;
    RETURN 'ok';
END
$$;

-- An administrator write of the account display name. The same rule as `auth_admin_set_password`.
CREATE OR REPLACE FUNCTION auth_admin_set_display_name(account_id uuid, new_display_name text)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    current_tenant uuid := NULLIF(current_setting('restichub.tenant_id', true), '')::uuid;
BEGIN
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.memberships
        WHERE user_id = account_id AND tenant_id = current_tenant
    ) THEN
        RETURN 'not_found';
    END IF;
    IF current_tenant <> '00000000-0000-7000-8000-000000000000'::uuid
       AND current_setting('restichub.scope', true) IS DISTINCT FROM 'platform'
       AND EXISTS (
           SELECT 1 FROM public.users WHERE id = account_id AND platform_admin
       ) THEN
        RETURN 'forbidden';
    END IF;
    IF current_tenant <> '00000000-0000-7000-8000-000000000000'::uuid
       AND current_setting('restichub.scope', true) IS DISTINCT FROM 'platform'
       AND EXISTS (
           SELECT 1
           FROM public.memberships AS member
           JOIN public.tenants AS tenant ON tenant.id = member.tenant_id
           WHERE member.user_id = account_id
             AND member.tenant_id <> current_tenant
             AND tenant.status <> 'deleted'
       ) THEN
        RETURN 'shared';
    END IF;
    UPDATE public.users
    SET display_name = new_display_name, updated_at = now()
    WHERE id = account_id;
    RETURN 'ok';
END
$$;

-- The member reads also return the Platform Admin right, so a tenant Admin refuses the write before
-- the database guard (ADR-0029).
DROP FUNCTION auth_tenant_members();

CREATE FUNCTION auth_tenant_members()
RETURNS TABLE (
    user_id uuid,
    username text,
    display_name text,
    role text,
    disabled boolean,
    account_disabled boolean,
    shared boolean,
    last_used_tenant_id uuid,
    platform_admin boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.display_name, member.role, member.disabled,
           account.disabled,
           EXISTS (
               SELECT 1
               FROM public.memberships AS other
               JOIN public.tenants AS other_tenant ON other_tenant.id = other.tenant_id
               WHERE other.user_id = account.id
                 AND other.tenant_id <> member.tenant_id
                 AND other_tenant.status <> 'deleted'
           ),
           account.last_used_tenant_id,
           account.platform_admin
    FROM public.memberships AS member
    JOIN public.users AS account ON account.id = member.user_id
    WHERE member.tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
    ORDER BY account.username
$$;

DROP FUNCTION auth_tenant_member(uuid);

CREATE FUNCTION auth_tenant_member(account_id uuid)
RETURNS TABLE (
    user_id uuid,
    username text,
    display_name text,
    role text,
    disabled boolean,
    account_disabled boolean,
    shared boolean,
    last_used_tenant_id uuid,
    platform_admin boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.display_name, member.role, member.disabled,
           account.disabled,
           EXISTS (
               SELECT 1
               FROM public.memberships AS other
               JOIN public.tenants AS other_tenant ON other_tenant.id = other.tenant_id
               WHERE other.user_id = account.id
                 AND other.tenant_id <> member.tenant_id
                 AND other_tenant.status <> 'deleted'
           ),
           account.last_used_tenant_id,
           account.platform_admin
    FROM public.memberships AS member
    JOIN public.users AS account ON account.id = member.user_id
    WHERE member.tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
      AND member.user_id = account_id
    LIMIT 1
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

ALTER FUNCTION check_platform_admin_membership() OWNER TO restichub_auth;
ALTER FUNCTION auth_admin_set_password(uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_admin_set_display_name(uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_tenant_members() OWNER TO restichub_auth;
ALTER FUNCTION auth_tenant_member(uuid) OWNER TO restichub_auth;
ALTER FUNCTION tenant_admin_counts() OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account(text) OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account_by_id(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_accounts() OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_create_account(uuid, text, text, text, text, boolean) OWNER TO restichub_auth;
ALTER FUNCTION auth_create_platform_admin(uuid, text, text, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_platform_admin_count() OWNER TO restichub_auth;
ALTER FUNCTION auth_is_platform_admin(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_platform_admin(uuid, boolean) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION check_platform_admin_membership() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_admin_set_password(uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_admin_set_display_name(uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_tenant_members() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_tenant_member(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION tenant_admin_counts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account_by_id(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_accounts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_create_account(uuid, text, text, text, text, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_create_platform_admin(uuid, text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_platform_admin_count() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_is_platform_admin(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_platform_admin(uuid, boolean) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION auth_admin_set_password(uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_admin_set_display_name(uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_tenant_members() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_tenant_member(uuid) TO restichub_app;
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
