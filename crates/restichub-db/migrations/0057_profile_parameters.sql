-- The parameters of a profile, with their values on a host and on a plan (FR-PF-09 to FR-PF-12,
-- ADR-0047).
--
-- A profile declares parameters: a name, a kind (`text` or `secret`), a required flag, a default
-- value for a `text` parameter and a description. The declaration is part of the profile JSON, so a
-- saved profile carries its own contract and a rename becomes a new parameter.
--
-- A host holds one value for each parameter name, and every profile of the host reads it. A plan
-- overrides a host value for its own run. The control plane resolves a value in the order plan,
-- host, default. A `secret` value is a reference to a secret of the purpose `hook_secret`; the
-- ciphertext stays in the `secrets` table, so a table dump of the values discloses no secret.
--
-- The kind decides which column holds the value, so a row cannot hold both and cannot hold
-- neither. The foreign key to the secret refuses the delete of a secret that a value names: a
-- dangling reference would stop every job of the plan or of the host.

ALTER TABLE profiles ADD COLUMN parameters jsonb NOT NULL DEFAULT '[]';

ALTER TABLE profiles ADD CONSTRAINT profiles_parameters_array_check
    CHECK (jsonb_typeof(parameters) = 'array');

CREATE TABLE agent_parameter_values (
    tenant_id uuid NOT NULL,
    agent_id uuid NOT NULL,
    name text NOT NULL,
    kind text NOT NULL,
    value text,
    secret_id uuid,
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid,
    PRIMARY KEY (tenant_id, agent_id, name),
    CONSTRAINT agent_parameter_values_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT agent_parameter_values_agent_fk FOREIGN KEY (tenant_id, agent_id)
        REFERENCES agents (tenant_id, id),
    CONSTRAINT agent_parameter_values_secret_fk FOREIGN KEY (tenant_id, secret_id)
        REFERENCES secrets (tenant_id, id) ON DELETE RESTRICT,
    CONSTRAINT agent_parameter_values_updated_by_fk FOREIGN KEY (updated_by) REFERENCES users (id),
    CONSTRAINT agent_parameter_values_kind_check CHECK (kind IN ('text', 'secret')),
    CONSTRAINT agent_parameter_values_value_check CHECK (
        (kind = 'text' AND value IS NOT NULL AND secret_id IS NULL)
        OR (kind = 'secret' AND value IS NULL AND secret_id IS NOT NULL)
    )
);

-- The delete of a secret and the usage list read the referencing rows by the secret.
CREATE INDEX agent_parameter_values_secret_idx ON agent_parameter_values (secret_id)
    WHERE secret_id IS NOT NULL;

ALTER TABLE agent_parameter_values ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_parameter_values FORCE ROW LEVEL SECURITY;
CREATE POLICY agent_parameter_values_tenant ON agent_parameter_values
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON agent_parameter_values TO restichub_app;

CREATE TABLE plan_parameter_values (
    tenant_id uuid NOT NULL,
    plan_id uuid NOT NULL,
    name text NOT NULL,
    kind text NOT NULL,
    value text,
    secret_id uuid,
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid,
    PRIMARY KEY (tenant_id, plan_id, name),
    CONSTRAINT plan_parameter_values_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    -- The delete of a plan removes its overrides: they have no meaning without the plan.
    CONSTRAINT plan_parameter_values_plan_fk FOREIGN KEY (tenant_id, plan_id)
        REFERENCES plans (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT plan_parameter_values_secret_fk FOREIGN KEY (tenant_id, secret_id)
        REFERENCES secrets (tenant_id, id) ON DELETE RESTRICT,
    CONSTRAINT plan_parameter_values_updated_by_fk FOREIGN KEY (updated_by) REFERENCES users (id),
    CONSTRAINT plan_parameter_values_kind_check CHECK (kind IN ('text', 'secret')),
    CONSTRAINT plan_parameter_values_value_check CHECK (
        (kind = 'text' AND value IS NOT NULL AND secret_id IS NULL)
        OR (kind = 'secret' AND value IS NULL AND secret_id IS NOT NULL)
    )
);

CREATE INDEX plan_parameter_values_secret_idx ON plan_parameter_values (secret_id)
    WHERE secret_id IS NOT NULL;

ALTER TABLE plan_parameter_values ENABLE ROW LEVEL SECURITY;
ALTER TABLE plan_parameter_values FORCE ROW LEVEL SECURITY;
CREATE POLICY plan_parameter_values_tenant ON plan_parameter_values
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON plan_parameter_values TO restichub_app;
