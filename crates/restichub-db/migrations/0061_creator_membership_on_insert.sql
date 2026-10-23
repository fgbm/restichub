-- The creator-membership check runs on an insert and on a change of the creator only (FR-SC-12).
--
-- Before this migration every update of a row re-checked the membership of its creator. When an
-- Admin removed the membership of a user, the plans of that user could not be updated any more:
-- the scheduler could neither advance a plan nor record its failure, and the plan stayed due.
-- Migration 0056 already treats such a row as valid. The foreign key to `users` still holds.

CREATE OR REPLACE FUNCTION check_creator_membership() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.created_by IS NOT NULL
       AND (TG_OP = 'INSERT' OR NEW.created_by IS DISTINCT FROM OLD.created_by)
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
