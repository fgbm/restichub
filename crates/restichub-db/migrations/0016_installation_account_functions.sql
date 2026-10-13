-- The installation-wide account lookups and the member reads (ADR-0025, issue #44).
--
-- An account row lives in the platform tenant, so a tenant transaction reads no account row
-- directly. Every account read and every account write that a tenant Admin makes runs through a
-- narrow `SECURITY DEFINER` function owned by `restichub_auth`, which has `BYPASSRLS`. The
-- functions follow the rules of migration 0008: each sets the search path, every relation is
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
GRANT SELECT, INSERT, UPDATE, DELETE ON users TO restichub_auth;
GRANT SELECT, DELETE ON sessions TO restichub_auth;

-- The per-tenant user count becomes the count of the memberships of the tenant (ADR-0025).
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

-- Finds the account by the username of the installation, without the letter case.
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

-- Finds the account by the identifier.
CREATE FUNCTION auth_find_account_by_id(account_id uuid)
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
    WHERE account.id = account_id
    LIMIT 1
$$;

-- Lists the active memberships of an account in active tenants, ordered by the tenant name.
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

-- True when the account has an active membership in an active tenant other than the given one.
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
          AND NOT member.disabled
          AND tenant.status = 'active'
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
                 AND NOT other.disabled
                 AND other_tenant.status = 'active'
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
                 AND NOT other.disabled
                 AND other_tenant.status = 'active'
           ),
           account.last_used_tenant_id
    FROM public.memberships AS member
    JOIN public.users AS account ON account.id = member.user_id
    WHERE member.tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid
      AND member.user_id = account_id
    LIMIT 1
$$;

-- Creates an account in the platform tenant. The id comes from the caller (UUIDv7).
CREATE FUNCTION auth_create_account(
    new_id uuid,
    account_name text,
    new_password_hash text,
    new_display_name text
)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    INSERT INTO public.users (id, tenant_id, username, password_hash, display_name)
    VALUES (
        new_id,
        '00000000-0000-7000-8000-000000000000'::uuid,
        account_name,
        new_password_hash,
        new_display_name
    )
$$;

CREATE FUNCTION auth_set_account_password(new_id uuid, new_password_hash text)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    UPDATE public.users
    SET password_hash = new_password_hash, updated_at = now()
    WHERE id = new_id;
    RETURN FOUND;
END
$$;

CREATE FUNCTION auth_set_account_display_name(new_id uuid, new_display_name text)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    UPDATE public.users
    SET display_name = new_display_name, updated_at = now()
    WHERE id = new_id;
    RETURN FOUND;
END
$$;

-- Disables or enables the account itself. Only a Platform Admin calls this.
CREATE FUNCTION auth_set_account_disabled(new_id uuid, new_disabled boolean)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    UPDATE public.users
    SET disabled = new_disabled, updated_at = now()
    WHERE id = new_id;
    RETURN FOUND;
END
$$;

CREATE FUNCTION auth_set_last_tenant(new_id uuid, new_tenant_id uuid)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    UPDATE public.users
    SET last_used_tenant_id = new_tenant_id, updated_at = now()
    WHERE id = new_id
$$;

-- Deletes the sessions of an account. A password change keeps the current session.
CREATE FUNCTION auth_delete_account_sessions(new_id uuid, keep_session uuid)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    deleted bigint;
BEGIN
    DELETE FROM public.sessions
    WHERE user_id = new_id
      AND (keep_session IS NULL OR id <> keep_session);
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END
$$;

ALTER FUNCTION tenant_user_counts() OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account(text) OWNER TO restichub_auth;
ALTER FUNCTION auth_find_account_by_id(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_list_memberships(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_account_is_shared(uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_tenant_members() OWNER TO restichub_auth;
ALTER FUNCTION auth_tenant_member(uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_create_account(uuid, text, text, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_account_password(uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_account_display_name(uuid, text) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_account_disabled(uuid, boolean) OWNER TO restichub_auth;
ALTER FUNCTION auth_set_last_tenant(uuid, uuid) OWNER TO restichub_auth;
ALTER FUNCTION auth_delete_account_sessions(uuid, uuid) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION tenant_user_counts() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_find_account_by_id(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_list_memberships(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_account_is_shared(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_tenant_members() FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_tenant_member(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_create_account(uuid, text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_account_password(uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_account_display_name(uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_account_disabled(uuid, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_set_last_tenant(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION auth_delete_account_sessions(uuid, uuid) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION tenant_user_counts() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_account(text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_find_account_by_id(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_list_memberships(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_account_is_shared(uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_tenant_members() TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_tenant_member(uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_create_account(uuid, text, text, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_set_account_password(uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_set_account_display_name(uuid, text) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_set_account_disabled(uuid, boolean) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_set_last_tenant(uuid, uuid) TO restichub_app;
GRANT EXECUTE ON FUNCTION auth_delete_account_sessions(uuid, uuid) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
