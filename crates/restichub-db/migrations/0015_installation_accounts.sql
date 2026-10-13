-- Installation-wide user accounts (ADR-0025, issue #44).
--
-- A user account becomes installation-wide. Every account row moves to the platform tenant and the
-- username becomes unique in the installation, without the letter case. A membership stays in its
-- tenant and refers to the account by `user_id`. The membership gains a `disabled` flag and the
-- account gains `last_used_tenant_id`.
--
-- The account rows keep `tenant_id`, so ADR-0006 holds and the RLS of every table stays. A tenant
-- transaction reads no account row directly: it uses the SECURITY DEFINER functions of migration
-- 0016.
--
-- The migration merges nothing. A username that exists in two tenants stops the migration with a
-- message that names the two tenants and the username.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

-- The owner reads every row for the collision check and the move. The owner login is subject to
-- FORCE ROW LEVEL SECURITY, so the check and the move run with the force off for `users`.
ALTER TABLE users NO FORCE ROW LEVEL SECURITY;

-- A username that exists in two tenants stops the migration and names the two tenants.
DO $$
DECLARE
    duplicate record;
    tenant_names text;
BEGIN
    FOR duplicate IN
        SELECT lower(account.username) AS name
        FROM users AS account
        GROUP BY lower(account.username)
        HAVING count(DISTINCT account.tenant_id) > 1
    LOOP
        SELECT string_agg(tenant.name, ' and ' ORDER BY tenant.name)
        INTO tenant_names
        FROM (
            SELECT DISTINCT account.tenant_id
            FROM users AS account
            WHERE lower(account.username) = duplicate.name
        ) AS holder
        JOIN tenants AS tenant ON tenant.id = holder.tenant_id;
        RAISE EXCEPTION
            'the username % exists in the tenants %; an installation-wide account needs one account for the name (ADR-0025)',
            duplicate.name,
            tenant_names;
    END LOOP;
END
$$;

-- The account foreign keys change: an account lives in the platform tenant, so the composite
-- `(tenant_id, user_id)` no longer matches. A membership, a session, a token and a secret refer to
-- the account by `user_id` alone.
ALTER TABLE memberships DROP CONSTRAINT memberships_user_fk;
ALTER TABLE sessions DROP CONSTRAINT sessions_user_fk;
ALTER TABLE api_tokens DROP CONSTRAINT api_tokens_created_by_fk;
ALTER TABLE secrets DROP CONSTRAINT secrets_created_by_fk;

-- The account name is unique in the installation, without the letter case.
DROP INDEX users_tenant_username_lower_unique;
CREATE UNIQUE INDEX users_username_lower_unique ON users (lower(username));

-- The account holds the tenant that was last used through it.
ALTER TABLE users
    ADD COLUMN last_used_tenant_id uuid,
    ADD CONSTRAINT users_last_used_tenant_fk FOREIGN KEY (last_used_tenant_id)
        REFERENCES tenants (id);

-- Every account row moves to the platform tenant and keeps its id.
UPDATE users
SET tenant_id = '00000000-0000-7000-8000-000000000000';

-- A membership can be disabled for one tenant without touching the account.
ALTER TABLE memberships ADD COLUMN disabled boolean NOT NULL DEFAULT false;

-- The account references.
ALTER TABLE memberships
    ADD CONSTRAINT memberships_user_fk FOREIGN KEY (user_id) REFERENCES users (id);
ALTER TABLE sessions
    ADD CONSTRAINT sessions_user_fk FOREIGN KEY (user_id) REFERENCES users (id);
ALTER TABLE api_tokens
    ADD CONSTRAINT api_tokens_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id);
ALTER TABLE secrets
    ADD CONSTRAINT secrets_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id);

-- The move keeps the forced Row-Level Security of the account table (AC-TN-04).
ALTER TABLE users FORCE ROW LEVEL SECURITY;
