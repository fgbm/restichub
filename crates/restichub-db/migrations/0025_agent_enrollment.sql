-- The enrollment exchange (ADR-0026, ADR-0032, FR-ID-01, FR-ID-02, FR-ID-05, FR-TN-03).
--
-- The enrollment endpoint reads the token before it knows the tenant, and every token is under Row
-- Level Security. A narrow SECURITY DEFINER function owned by `restichub_auth` (BYPASSRLS) consumes
-- the token by its hash across the installation. It returns the tenant, the token identifier and
-- the labels of a valid token, or the refusal reason with the same tenant, so the caller records
-- the refusal in the right tenant. The caller then runs in the tenant transaction of that tenant
-- and creates the agent row (FR-TN-03).
--
-- The function follows the rules of migration 0008: each sets the search path, every relation is
-- schema-qualified, and the runtime role `restichub_app` gets `EXECUTE` and nothing else.
--
-- The consume and the agent insert run in one transaction of the caller, so a consumed token never
-- stays without an agent. The function takes the row lock (`FOR UPDATE`), so two requests with one
-- token serialize and the second sees `used_at`.

-- The registry row records the host facts and the labels of the token (task #48).
ALTER TABLE agents ADD COLUMN restic_version text NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN labels text[] NOT NULL DEFAULT '{}';

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

-- The definer role reads and consumes the token rows of every tenant.
GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;
GRANT SELECT, UPDATE ON enrollment_tokens TO restichub_auth;

CREATE FUNCTION enroll_consume_token(token bytea)
RETURNS TABLE (outcome text, tenant_id uuid, token_id uuid, labels text[])
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    found_id uuid;
    found_tenant uuid;
    found_labels text[];
    found_used timestamptz;
    found_revoked timestamptz;
    found_expires timestamptz;
BEGIN
    SELECT entry.id, entry.tenant_id, entry.labels, entry.used_at, entry.revoked_at, entry.expires_at
      INTO found_id, found_tenant, found_labels, found_used, found_revoked, found_expires
      FROM public.enrollment_tokens AS entry
     WHERE entry.token_hash = token
       FOR UPDATE;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    IF found_used IS NOT NULL THEN
        RETURN QUERY SELECT 'used'::text, found_tenant, found_id, found_labels;
        RETURN;
    END IF;
    IF found_revoked IS NOT NULL THEN
        RETURN QUERY SELECT 'revoked'::text, found_tenant, found_id, found_labels;
        RETURN;
    END IF;
    IF found_expires <= now() THEN
        RETURN QUERY SELECT 'expired'::text, found_tenant, found_id, found_labels;
        RETURN;
    END IF;
    UPDATE public.enrollment_tokens AS entry SET used_at = now() WHERE entry.id = found_id;
    RETURN QUERY SELECT 'consumed'::text, found_tenant, found_id, found_labels;
END
$$;

ALTER FUNCTION enroll_consume_token(bytea) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION enroll_consume_token(bytea) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION enroll_consume_token(bytea) TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
