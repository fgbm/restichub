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

-- Server scripts on trusted hosts (FR-HK-08 to FR-HK-12, ADR-0037).
--
-- `agents.server_scripts` and `agents.script_interpreters` hold the last report of the local trust
-- mode. `profiles.scripts` holds the ordered scripts of a profile, each with its body, its
-- interpreter, its phase and its typed environment variables (a plain value or a secret id).
--
-- `script_templates` and `job_templates` belong to a tenant. A script template holds a body with
-- declared parameters. A job template holds an ordered set of scripts from which a profile is made.
-- The built-in templates ship with the control plane and have no row.
--
-- `job_planned_steps` also keeps the kind, the name and the phase of each step, so the job detail
-- lists every script in order before it runs (FR-HK-11).

ALTER TABLE agents ADD COLUMN server_scripts text NOT NULL DEFAULT 'unknown';
ALTER TABLE agents ADD COLUMN script_interpreters text[] NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD CONSTRAINT agents_server_scripts_check CHECK (
    server_scripts IN ('unknown', 'disabled', 'enabled', 'insecure')
);

ALTER TABLE profiles ADD COLUMN scripts jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE profiles ADD CONSTRAINT profiles_scripts_array_check CHECK (
    jsonb_typeof(scripts) = 'array'
);

ALTER TABLE job_planned_steps ADD COLUMN kind text NOT NULL DEFAULT '';
ALTER TABLE job_planned_steps ADD COLUMN name text NOT NULL DEFAULT '';
ALTER TABLE job_planned_steps ADD COLUMN phase text NOT NULL DEFAULT '';
ALTER TABLE job_planned_steps ADD CONSTRAINT job_planned_steps_name_check CHECK (
    char_length(name) <= 128 AND char_length(kind) <= 32 AND char_length(phase) <= 16
);

CREATE TABLE script_templates (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    interpreter text NOT NULL,
    phase text NOT NULL DEFAULT 'before',
    body text NOT NULL,
    timeout_seconds integer NOT NULL DEFAULT 0,
    failure_policy text NOT NULL DEFAULT 'abort_backup',
    -- The declared parameters: an array of name, label, type and default.
    parameters jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT script_templates_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT script_templates_tenant_id_unique UNIQUE (tenant_id, id),
    CONSTRAINT script_templates_name_unique UNIQUE (tenant_id, name),
    CONSTRAINT script_templates_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id),
    CONSTRAINT script_templates_interpreter_check CHECK (
        interpreter IN ('powershell', 'cmd', 'bash', 'sh')
    ),
    CONSTRAINT script_templates_phase_check CHECK (
        phase IN ('before', 'after', 'on_success', 'on_error', 'finally')
    ),
    CONSTRAINT script_templates_policy_check CHECK (failure_policy IN ('abort_backup', 'continue')),
    CONSTRAINT script_templates_size_check CHECK (
        char_length(name) BETWEEN 1 AND 64 AND char_length(description) <= 512
        AND octet_length(body) BETWEEN 1 AND 65536
        AND timeout_seconds BETWEEN 0 AND 86400
        AND jsonb_typeof(parameters) = 'array'
    )
);

CREATE INDEX script_templates_tenant_idx ON script_templates (tenant_id, name);

CREATE TRIGGER script_templates_creator_membership
    BEFORE INSERT OR UPDATE ON script_templates
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

CREATE TABLE job_templates (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    -- The ordered scripts: the same shape as `profiles.scripts`.
    scripts jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT job_templates_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT job_templates_tenant_id_unique UNIQUE (tenant_id, id),
    CONSTRAINT job_templates_name_unique UNIQUE (tenant_id, name),
    CONSTRAINT job_templates_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id),
    CONSTRAINT job_templates_size_check CHECK (
        char_length(name) BETWEEN 1 AND 64 AND char_length(description) <= 512
        AND jsonb_typeof(scripts) = 'array'
    )
);

CREATE INDEX job_templates_tenant_idx ON job_templates (tenant_id, name);

CREATE TRIGGER job_templates_creator_membership
    BEFORE INSERT OR UPDATE ON job_templates
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

ALTER TABLE script_templates ENABLE ROW LEVEL SECURITY;
ALTER TABLE script_templates FORCE ROW LEVEL SECURITY;
CREATE POLICY script_templates_tenant ON script_templates
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE job_templates ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_templates FORCE ROW LEVEL SECURITY;
CREATE POLICY job_templates_tenant ON job_templates
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON script_templates TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON job_templates TO restichub_app;
