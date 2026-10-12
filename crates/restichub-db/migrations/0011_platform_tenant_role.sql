-- The platform tenant allows the platform_admin role only (issue #18, FR-TN-02).
--
-- The roles Admin, Operator and Viewer apply in one tenant. The platform tenant holds the Platform
-- Admin users and the platform audit records only. Migration 0003 and 0004 already forbid the role
-- `platform_admin` outside the platform tenant. This migration adds the other direction: inside the
-- platform tenant only `platform_admin` is valid.
--
-- The pre-check raises a clear error that names the offending table, so an installation with a bad
-- row stops with an actionable message instead of an opaque constraint error.

DO $$
DECLARE
    bad_memberships bigint;
    bad_tokens bigint;
BEGIN
    SELECT count(*) INTO bad_memberships
    FROM memberships
    WHERE tenant_id = '00000000-0000-7000-8000-000000000000'
      AND role <> 'platform_admin';
    IF bad_memberships > 0 THEN
        RAISE EXCEPTION
            'memberships: % row(s) of the platform tenant have a role other than platform_admin; fix the rows before this migration',
            bad_memberships;
    END IF;

    SELECT count(*) INTO bad_tokens
    FROM api_tokens
    WHERE tenant_id = '00000000-0000-7000-8000-000000000000'
      AND role <> 'platform_admin';
    IF bad_tokens > 0 THEN
        RAISE EXCEPTION
            'api_tokens: % row(s) of the platform tenant have a role other than platform_admin; fix the rows before this migration',
            bad_tokens;
    END IF;
END
$$;

ALTER TABLE memberships
    ADD CONSTRAINT memberships_platform_tenant_role
    CHECK (
        tenant_id <> '00000000-0000-7000-8000-000000000000'
        OR role = 'platform_admin'
    );

ALTER TABLE api_tokens
    ADD CONSTRAINT api_tokens_platform_tenant_role
    CHECK (
        tenant_id <> '00000000-0000-7000-8000-000000000000'
        OR role = 'platform_admin'
    );
