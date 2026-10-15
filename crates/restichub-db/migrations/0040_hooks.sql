-- Hooks from the agent allow-list (FR-HK-01 to FR-HK-05, ADR-0004).
--
-- The agent reports the public part of its allow-list in `AgentHello`: the identifier, the phases
-- and the parameter schema of each hook, never a path or a fixed argument. The control plane keeps
-- the last report for the hook picker of the profile, for the save-time check of the profile
-- (AC-HK-05) and for the Host page. The report is a JSON object that maps a hook identifier to its
-- schema. The job builder reads it again at each dispatch, so a hook that the agent no longer
-- reports stops the job with a typed reason.
--
-- `hook_allow_list` tells why an agent reports no hook: no file, a file that the agent does not
-- trust (AC-HK-01) or a file that does not obey the format. `unknown` is an agent that does not
-- report the state.

ALTER TABLE agents ADD COLUMN hooks jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE agents ADD COLUMN hook_allow_list text NOT NULL DEFAULT 'unknown';
ALTER TABLE agents ADD CONSTRAINT agents_hook_allow_list_check CHECK (
    hook_allow_list IN ('unknown', 'absent', 'loaded', 'insecure', 'invalid')
);
ALTER TABLE agents ADD CONSTRAINT agents_hooks_object_check CHECK (jsonb_typeof(hooks) = 'object');
