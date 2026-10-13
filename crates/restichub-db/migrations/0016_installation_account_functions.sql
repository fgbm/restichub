-- The installation-wide account lookups and the member reads (ADR-0025, issue #44).
--
-- An account row lives in the platform tenant, so a tenant transaction reads no account row
-- directly. Every account read and every account write runs through a narrow `SECURITY DEFINER`
-- function owned by `restichub_auth`, which has `BYPASSRLS`. Each function carries its own scope
-- rule at the database level, so a caller cannot widen its reach through the API layer:
--
--   * an administrator write (password, display name) needs a membership in the current tenant and
--     refuses a shared account unless the current tenant is the platform tenant;
--   * the user's own password change is bound to the caller's own session;
--   * the account itself is disabled and its sessions are removed across tenants from the platform
--     tenant only;
--   * the last used tenant is written for the account of the caller's own session only.
--
-- The functions follow the rules of migration 0008: each sets the search path, every relation is
-- schema-qualified, and the runtime role `restichub_app` gets `EXECUTE` and nothing else.
--
-- `auth_find_tenant`, `auth_find_session` and `auth_find_api_token` stay from migration 0008.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

-- The definer role reads the account, the membership, the tenant and the session tables, and it
-- writes the account and the session rows that the tenant Admin and the Platform Admin change.
GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;
GRANT SELECT ON memberships, tenants TO restichub_auth;
GRANT INSERT, UPDATE ON memberships TO restichub_auth;
GRANT SELECT, INSERT, UPDATE, DELETE ON users TO restichub_auth;
GRANT SELECT, DELETE ON sessions TO restichub_auth;

-- The per-tenant user count is the count of the memberships of the tenant (ADR-0025).
CREATE OR REPLACE FUNCTION tenant_user_counts()
RETURNS TABLE (tenant_id uuid, user_count bigint)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT member.tenant_id, count(*)::bigint
    FROM public.memberships AS member
    WHERE current_setting('restichub.scope', true) = 'platform'
    GROUP BY member.tenant_id
$$;

-- Finds the account by the username of the installation, without the letter case. Sign-in verifies
-- the password against the hash, so this function returns it.
CREATE FUNCTION auth_find_account(account_name text)
RETURNS TABLE (
    id uuid,
    username text,
    password_hash text,
    display_name text,
    disabled boolean,
    last_used_tenant_id uuid
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.password_hash, account.display_name,
           account.disabled, account.last_used_tenant_id
    FROM public.users AS account
    WHERE lower(account.username) = lower(account_name)
    LIMIT 1
$$;

-- Finds the account by the identifier. It returns no password hash.
CREATE FUNCTION auth_find_account_by_id(account_id uuid)
RETURNS TABLE (
    id uuid,
    username text,
    display_name text,
    disabled boolean,
    last_used_tenant_id uuid
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.id, account.username, account.display_name, account.disabled,
           account.last_used_tenant_id
    FROM public.users AS account
    WHERE account.id = account_id
    LIMIT 1
$$;

-- The password hash of the caller's own account, bound to the caller's own session in the current
-- tenant. It returns no row otherwise. The own password change uses it.
CREATE FUNCTION auth_self_password_hash(account_id uuid, session_id uuid)
RETURNS TABLE (password_hash text)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT account.password_hash
    FROM public.users AS account
    JOIN public.sessions AS session ON session.user_id = account.id
    WHERE account.id = account_id
      AND session.id = session_id
      AND session.tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
    LIMIT 1
$$;

-- Lists the active memberships of an account in active tenants, ordered by the tenant name. Sign-in
-- and the tenant list use it, so a suspended or a disabled membership is skipped.
CREATE FUNCTION auth_list_memberships(account_id uuid)
RETURNS TABLE (tenant_id uuid, tenant_name text, role text)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT member.tenant_id, tenant.name, member.role
    FROM public.memberships AS member
    JOIN public.tenants AS tenant ON tenant.id = member.tenant_id
    WHERE member.user_id = account_id
      AND NOT member.disabled
      AND tenant.status = 'active'
    ORDER BY tenant.name
$$;

-- True when the account holds any membership in a tenant other than the given one, that is not
-- deleted. A disabled membership and a suspended tenant still count: the account is shared with
-- that tenant (ADR-0025, threat T14).
CREATE FUNCTION auth_account_is_shared(account_id uuid, exclude_tenant_id uuid)
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1
        FROM public.memberships AS member
        JOIN public.tenants AS tenant ON tenant.id = member.tenant_id
        WHERE member.user_id = account_id
          AND member.tenant_id <> exclude_tenant_id
          AND tenant.status <> 'deleted'
    )
$$;

-- The members of the tenant of the transaction: the administrator read of the accounts of its
-- members. It returns only accounts with a membership in the current tenant.
CREATE FUNCTION auth_tenant_members()
RETURNS TABLE (
    user_id uuid,
    username text,
    display_name text,
    role text,
    disabled boolean,
    account_disabled boolean,
    shared boolean,
    last_used_tenant_id uuid
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
           account.last_used_tenant_id
    FROM public.memberships AS member
    JOIN public.users AS account ON account.id = member.user_id
    WHERE member.tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
    ORDER BY account.username
$$;

-- The member of the tenant of the transaction with the given id. `None` when the account holds no
-- membership in the tenant.
CREATE FUNCTION auth_tenant_member(account_id uuid)
RETURNS TABLE (
    user_id uuid,
    username text,
    display_name text,
    role text,
    disabled boolean,
    account_disabled boolean,
    shared boolean,
    last_used_tenant_id uuid
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
           account.last_used_tenant_id
    FROM public.memberships AS member
    JOIN public.users AS account ON account.id = member.user_id
    WHERE member.tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
      AND member.user_id = account_id
    LIMIT 1
$$;

-- Creates an account in the platform tenant. The id comes from the caller (UUIDv7). A create and
-- the membership that follows take the account row lock, so no administrator write races with them.
CREATE FUNCTION auth_create_account(
    new_id uuid,
    account_name text,
    new_password_hash text,
    new_display_name text
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    INSERT INTO public.users (id, tenant_id, username, password_hash, display_name)
    VALUES (
        new_id,
        '00000000-0000-7000-8000-000000000000'::uuid,
        account_name,
        new_password_hash,
        new_display_name
    );
    PERFORM 1 FROM public.users WHERE id = new_id FOR UPDATE;
END
$$;

-- Takes the account row lock. A membership insert calls it first, so it serializes with an
-- administrator write of the same account. `false` when the account is absent.
CREATE FUNCTION auth_lock_account(account_id uuid)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    RETURN FOUND;
END
$$;

-- An administrator write of the account password. The function takes the account row lock, checks
-- the membership in the current tenant and the sharing, then writes. The status is `ok`,
-- `not_found` (no membership in the current tenant) or `shared` (a membership in another tenant,
-- while the current tenant is not the platform tenant).
CREATE FUNCTION auth_admin_set_password(account_id uuid, new_password_hash text)
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

-- An administrator write of the account display name. The same scope rule as
-- `auth_admin_set_password`.
CREATE FUNCTION auth_admin_set_display_name(account_id uuid, new_display_name text)
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

-- The password change of the user itself. The function writes the new hash only when the given
-- session belongs to the account and to the current tenant. `false` otherwise.
CREATE FUNCTION auth_self_set_password(account_id uuid, session_id uuid, new_password_hash text)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    current_tenant uuid := NULLIF(current_setting('restichub.tenant_id', true), '')::uuid;
BEGIN
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.sessions
        WHERE id = session_id AND user_id = account_id AND tenant_id = current_tenant
    ) THEN
        RETURN false;
    END IF;
    UPDATE public.users
    SET password_hash = new_password_hash, updated_at = now()
    WHERE id = account_id;
    RETURN FOUND;
END
$$;

-- Disables or enables the account itself. Only the platform tenant or the platform scope may do
-- it. The status is `ok`, `not_found` or `forbidden`.
CREATE FUNCTION auth_set_account_disabled(account_id uuid, new_disabled boolean)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    current_tenant uuid := NULLIF(current_setting('restichub.tenant_id', true), '')::uuid;
BEGIN
    IF NOT (
        current_tenant = '00000000-0000-7000-8000-000000000000'::uuid
        OR current_setting('restichub.scope', true) = 'platform'
    ) THEN
        RETURN 'forbidden';
    END IF;
    UPDATE public.users
    SET disabled = new_disabled, updated_at = now()
    WHERE id = account_id;
    IF NOT FOUND THEN
        RETURN 'not_found';
    END IF;
    RETURN 'ok';
END
$$;

-- Deletes the sessions of an account. The whole installation is reached only by the platform tenant,
-- by the platform scope, or by the own password change (`keep_session` is a session of the same
-- account). Otherwise only the sessions of the current tenant are removed.
CREATE FUNCTION auth_delete_account_sessions(account_id uuid, keep_session uuid)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    deleted bigint;
    current_tenant uuid := NULLIF(current_setting('restichub.tenant_id', true), '')::uuid;
    own_session boolean;
BEGIN
    own_session := keep_session IS NOT NULL AND EXISTS (
        SELECT 1 FROM public.sessions WHERE id = keep_session AND user_id = account_id
    );
    IF current_setting('restichub.scope', true) = 'platform'
       OR current_tenant = '00000000-0000-7000-8000-000000000000'::uuid
       OR own_session THEN
        DELETE FROM public.sessions
        WHERE user_id = account_id AND (keep_session IS NULL OR id <> keep_session);
    ELSE
        DELETE FROM public.sessions
        WHERE user_id = account_id
          AND tenant_id = current_tenant
          AND (keep_session IS NULL OR id <> keep_session);
    END IF;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END
$$;

-- Deletes one session of the account. The tenant switch removes the source session through it.
CREATE FUNCTION auth_delete_session(session_id uuid, account_id uuid)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    deleted bigint;
BEGIN
    DELETE FROM public.sessions WHERE id = session_id AND user_id = account_id;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END
$$;

-- Stores the last used tenant, for the account of the caller's own session only. `false` when the
-- session does not belong to the account in the current tenant.
CREATE FUNCTION auth_set_last_tenant(account_id uuid, session_id uuid, new_tenant_id uuid)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    current_tenant uuid := NULLIF(current_setting('restichub.tenant_id', true), '')::uuid;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM public.sessions
        WHERE id = session_id AND user_id = account_id AND tenant_id = current_tenant
    ) THEN
        RETURN false;
    END IF;
    UPDATE public.users
    SET last_used_tenant_id = new_tenant_id, updated_at = now()
    WHERE id = account_id;
    RETURN FOUND;
END
$$;

-- Adds a membership for an existing account (Platform Admin only). The platform scope is required,
-- so a tenant transaction cannot add an account to a tenant. The function takes the account row
-- lock, so it serializes with an administrator write of the same account. It refuses the platform
-- tenant: a Platform Admin membership is created by the setup or by the platform user create only.
-- It re-enables a disabled membership and sets the role.
CREATE FUNCTION auth_add_membership(account_id uuid, target_tenant_id uuid, role text)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF current_setting('restichub.scope', true) <> 'platform' THEN
        RETURN false;
    END IF;
    PERFORM 1 FROM public.users WHERE id = account_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    IF target_tenant_id = '00000000-0000-7000-8000-000000000000'::uuid THEN
        RETURN false;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.tenants WHERE id = target_tenant_id AND status = 'active'
    ) THEN
        RETURN false;
    END IF;
    INSERT INTO public.memberships (tenant_id, user_id, role)
    VALUES (target_tenant_id, account_id, role)
    ON CONFLICT (tenant_id, user_id) DO UPDATE SET role = EXCLUDED.role, disabled = false;
    RETURN true;
END
$$;

ALTER FUNCTION tenant_user_counts() OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account(text) OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account_by_id(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_self_password_hash(uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_list_memberships(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_account_is_shared(uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_tenant_members() OWNER TO restichub_auth;
ALTER FUNCTION auth_tenant_member(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_create_account(uuid, text, text, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_lock_account(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_admin_set_password(uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_admin_set_display_name(uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_self_set_password(uuid, uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_account_disabled(uuid, boolean) OWNER TO restichub_auth;
ALTER FUNCTION auth_delete_account_sessions(uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_delete_session(uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_last_tenant(uuid, uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_add_membership(uuid, uuid, text) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION tenant_user_counts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account_by_id(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_self_password_hash(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_list_memberships(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_account_is_shared(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_tenant_members() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_tenant_member(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_create_account(uuid, text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_lock_account(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_admin_set_password(uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_admin_set_display_name(uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_self_set_password(uuid, uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_account_disabled(uuid, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_delete_account_sessions(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_delete_session(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_last_tenant(uuid, uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_add_membership(uuid, uuid, text) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION tenant_user_counts() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_account(text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_account_by_id(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_self_password_hash(uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_list_memberships(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_account_is_shared(uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_tenant_members() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_tenant_member(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_create_account(uuid, text, text, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_lock_account(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_admin_set_password(uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_admin_set_display_name(uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_self_set_password(uuid, uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_set_account_disabled(uuid, boolean) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_delete_account_sessions(uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_delete_session(uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_set_last_tenant(uuid, uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_add_membership(uuid, uuid, text) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
