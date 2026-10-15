-- The allow-list hooks do not exist any more (ADR-0040).
--
-- The agent no longer reports an allow-list, and a profile no longer holds a hook section. The
-- constraints of the columns go with them. A row of `job_planned_steps` with the kind `hook` stays
-- as history, so the kind check keeps the old value. The tables keep their `tenant_id` columns and
-- row-level security policies; this migration adds no table.
--
-- A dropped hook section is lost without a trace in the plan runs. So each profile that holds a
-- hook gets one audit record (action `profile_hooks_removed`, system actor) before the column goes.
-- The record keeps the hook ids and phases and no parameter values. The migration enters the
-- platform scope for the insert, as the policy `audit_log_platform_insert` requires.

DO $$
DECLARE
    removed record;
BEGIN
    PERFORM set_config('restichub.scope', 'platform', true);
    FOR removed IN
        SELECT id, tenant_id, name, hooks FROM profiles WHERE jsonb_array_length(hooks) > 0
    LOOP
        PERFORM set_config('restichub.audit_tenant_id', removed.tenant_id::text, true);
        INSERT INTO audit_log (id, tenant_id, actor_type, action, object_type, object_id, diff)
        VALUES (
            gen_random_uuid(),
            removed.tenant_id,
            'system',
            'profile_hooks_removed',
            'profile',
            removed.id,
            jsonb_build_object(
                'name', removed.name,
                'hooks', (
                    SELECT jsonb_agg(jsonb_build_object('hook_id', hook ->> 'hook_id', 'phase', hook ->> 'phase'))
                    FROM jsonb_array_elements(removed.hooks) AS hook
                )
            )
        );
    END LOOP;
    PERFORM set_config('restichub.audit_tenant_id', '', true);
    PERFORM set_config('restichub.scope', '', true);
END
$$;

ALTER TABLE agents DROP COLUMN hooks;
ALTER TABLE agents DROP COLUMN hook_allow_list;

ALTER TABLE profiles DROP COLUMN hooks;
