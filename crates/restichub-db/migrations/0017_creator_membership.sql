-- The creator of an API token and of a secret must be a member of the row's tenant (ADR-0025).
--
-- Before the installation-wide account model, a composite foreign key
-- `(tenant_id, created_by) REFERENCES users (tenant_id, id)` tied the creator to the tenant. The
-- account row now lives in the platform tenant, so that key is gone. This trigger restores the
-- guard at the database level: the creator must hold a membership in the tenant of the row.
--
-- The trigger runs as the caller, so the membership read is filtered by Row-Level Security to the
-- current tenant. The `WITH CHECK` of the row policy already pins `tenant_id` to the current tenant,
-- so the check reads the memberships of the tenant of the row.

CREATE FUNCTION check_creator_membership() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.created_by IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM public.memberships AS member
        WHERE member.user_id = NEW.created_by
          AND member.tenant_id = NEW.tenant_id
    ) THEN
        RAISE EXCEPTION
            'the creator % holds no membership in the tenant % of the row',
            NEW.created_by,
            NEW.tenant_id
            USING ERRCODE = '23503';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER api_tokens_creator_membership
    BEFORE INSERT OR UPDATE ON api_tokens
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

CREATE TRIGGER secrets_creator_membership
    BEFORE INSERT OR UPDATE ON secrets
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();
