-- The repositories (FR-RP-01, FR-RP-08, ADR-0003, ADR-0022, ADR-0023).
--
-- One row per repository, under the tenant policy of ADR-0023. The row holds the backend settings,
-- the identifiers of the bound secrets and the maintenance agent (FR-RP-08).
--
-- The backend is a closed set. Each backend uses its own group of columns; there is no free-form
-- map (the spirit of FR-PR-01). The check constraint refuses a row that holds a setting of another
-- backend.
--
-- A secret and a maintenance agent of another tenant must be impossible. The composite foreign keys
-- need a unique key on `(tenant_id, id)` of `secrets` and of `agents`; the migration adds them.

-- The composite keys for the tenant-scoped foreign keys below.
ALTER TABLE secrets ADD CONSTRAINT secrets_tenant_id_unique UNIQUE (tenant_id, id);
ALTER TABLE agents ADD CONSTRAINT agents_tenant_id_unique UNIQUE (tenant_id, id);

CREATE TABLE repositories (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    backend text NOT NULL,
    -- The S3-compatible backend.
    s3_endpoint text,
    s3_bucket text,
    s3_prefix text,
    s3_region text,
    -- The rest-server backend.
    rest_endpoint text,
    -- The local path backend.
    local_path text,
    -- The repository password and the backend credentials, by identifier (ADR-0017).
    password_secret_id uuid NOT NULL,
    s3_access_key_secret_id uuid,
    s3_secret_key_secret_id uuid,
    rest_password_secret_id uuid,
    -- The agent that runs forget, prune and check (FR-RP-08).
    maintenance_agent_id uuid,
    -- The repository becomes initialized through the agent (later); the control plane sets it true
    -- after a successful init step.
    initialized boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT repositories_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT repositories_tenant_name_unique UNIQUE (tenant_id, name),
    CONSTRAINT repositories_password_secret_fk FOREIGN KEY (tenant_id, password_secret_id)
        REFERENCES secrets (tenant_id, id),
    CONSTRAINT repositories_s3_access_secret_fk FOREIGN KEY (tenant_id, s3_access_key_secret_id)
        REFERENCES secrets (tenant_id, id),
    CONSTRAINT repositories_s3_secret_secret_fk FOREIGN KEY (tenant_id, s3_secret_key_secret_id)
        REFERENCES secrets (tenant_id, id),
    CONSTRAINT repositories_rest_password_secret_fk FOREIGN KEY (tenant_id, rest_password_secret_id)
        REFERENCES secrets (tenant_id, id),
    CONSTRAINT repositories_maintenance_agent_fk FOREIGN KEY (tenant_id, maintenance_agent_id)
        REFERENCES agents (tenant_id, id),
    CONSTRAINT repositories_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id),
    CONSTRAINT repositories_backend_check CHECK (backend IN ('s3', 'rest', 'local')),
    CONSTRAINT repositories_backend_settings_check CHECK (
        (
            backend = 's3'
            AND s3_endpoint IS NOT NULL AND s3_bucket IS NOT NULL
            AND s3_access_key_secret_id IS NOT NULL AND s3_secret_key_secret_id IS NOT NULL
            AND rest_endpoint IS NULL AND local_path IS NULL AND rest_password_secret_id IS NULL
        )
        OR (
            backend = 'rest'
            AND rest_endpoint IS NOT NULL
            AND s3_endpoint IS NULL AND s3_bucket IS NULL AND s3_prefix IS NULL
            AND s3_region IS NULL AND s3_access_key_secret_id IS NULL
            AND s3_secret_key_secret_id IS NULL AND local_path IS NULL
        )
        OR (
            backend = 'local'
            AND local_path IS NOT NULL
            AND s3_endpoint IS NULL AND s3_bucket IS NULL AND s3_prefix IS NULL AND s3_region IS NULL
            AND rest_endpoint IS NULL AND s3_access_key_secret_id IS NULL
            AND s3_secret_key_secret_id IS NULL AND rest_password_secret_id IS NULL
        )
    )
);

CREATE INDEX repositories_tenant_created_idx ON repositories (tenant_id, created_at DESC, id DESC);

-- The account row lives in the platform tenant, so the creator is guarded by the same trigger as
-- the secrets and the API tokens (ADR-0025, migration 0018).
CREATE TRIGGER repositories_creator_membership
    BEFORE INSERT OR UPDATE ON repositories
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

ALTER TABLE repositories ENABLE ROW LEVEL SECURITY;
ALTER TABLE repositories FORCE ROW LEVEL SECURITY;
CREATE POLICY repositories_tenant ON repositories
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON repositories TO restichub_app;
