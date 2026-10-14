-- The jobs (FR-PR-01, FR-PR-03, FR-PR-05, FR-RP-02, FR-RP-08, ADR-0003, ADR-0023).
--
-- One row per job, under the tenant policy of ADR-0023. The row is the seed of the scheduler of
-- epic #7: it holds the general lifecycle of a job, not the init-specific detail. The kind is a
-- closed set of the MVP step groups; the scheduler of #7 extends it.
--
-- The row never holds a secret. The repository password and the backend credentials live in the
-- encrypted `secrets` table and go with the signed job only (FR-PR-05, ADR-0003). The
-- `result_message` column holds a machine-readable code, never a secret and never prose
-- (NFR-DOC-04).
--
-- The job belongs to a tenant and names an agent and, for a repository job, a repository. The
-- composite foreign keys keep the agent and the repository in the same tenant.

-- The composite keys for the tenant-scoped foreign keys below come from earlier migrations:
-- `agents_tenant_id_unique` (0026) and `repositories_tenant_id_unique` (0027).

CREATE TABLE jobs (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    kind text NOT NULL,
    -- The agent that runs the job (FR-PR-03).
    agent_id uuid NOT NULL,
    -- The repository of the job. Null for a job that touches no repository.
    repository_id uuid,
    status text NOT NULL,
    -- The typed result of a finished job. Null until the job finishes.
    result_code text,
    -- A machine-readable error code from the agent. Never a secret and never prose.
    result_message text,
    -- The time after which the agent rejects the job (FR-SC-07).
    expires_at timestamptz,
    sent_at timestamptz,
    started_at timestamptz,
    finished_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT jobs_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT jobs_agent_fk FOREIGN KEY (tenant_id, agent_id) REFERENCES agents (tenant_id, id),
    CONSTRAINT jobs_repository_fk FOREIGN KEY (tenant_id, repository_id)
        REFERENCES repositories (tenant_id, id),
    CONSTRAINT jobs_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id),
    CONSTRAINT jobs_kind_check CHECK (
        kind IN ('init', 'test_access', 'backup', 'check', 'forget', 'prune', 'restore', 'unlock', 'other')
    ),
    CONSTRAINT jobs_status_check CHECK (
        status IN ('queued', 'sent', 'running', 'succeeded', 'failed', 'expired')
    ),
    CONSTRAINT jobs_result_code_check CHECK (
        result_code IS NULL
        OR result_code IN ('succeeded', 'failed', 'config_error', 'not_initialized', 'cancelled')
    )
);

CREATE INDEX jobs_tenant_created_idx ON jobs (tenant_id, created_at DESC, id DESC);
CREATE INDEX jobs_tenant_agent_idx ON jobs (tenant_id, agent_id, created_at DESC);

-- The account row lives in the platform tenant, so the creator is guarded by the same trigger as
-- the repositories and the secrets (ADR-0025, migration 0018).
CREATE TRIGGER jobs_creator_membership
    BEFORE INSERT OR UPDATE ON jobs
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

ALTER TABLE jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE jobs FORCE ROW LEVEL SECURITY;
CREATE POLICY jobs_tenant ON jobs
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON jobs TO restichub_app;
