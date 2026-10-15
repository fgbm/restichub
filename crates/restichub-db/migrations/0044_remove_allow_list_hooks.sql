-- The allow-list hooks do not exist any more (ADR-0040).
--
-- The agent no longer reports an allow-list, and a profile no longer holds a hook section. The
-- constraints of the columns go with them. A row of `job_planned_steps` with the kind `hook` stays
-- as history, so the kind check keeps the old value. The tables keep their `tenant_id` columns and
-- row-level security policies; this migration adds no table.

ALTER TABLE agents DROP COLUMN hooks;
ALTER TABLE agents DROP COLUMN hook_allow_list;

ALTER TABLE profiles DROP COLUMN hooks;
