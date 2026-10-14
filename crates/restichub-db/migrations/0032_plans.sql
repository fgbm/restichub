-- The plans and the scheduled jobs (FR-SC-01, FR-SC-02, FR-SC-07, D8, ADR-0010, ADR-0023).
--
-- A plan binds a profile to one agent with a cron schedule and a time zone (FR-SC-01, FR-SC-11).
-- The scheduler of the control plane reads the due rows and creates one job for each fire time
-- (FR-SC-02). The row holds no secret and no compiled command: a job names the plan and the fire
-- time only, and the control plane recompiles the stored profile through the compiler when it
-- dispatches (a later part of epic #7).
--
-- The schedule belongs to the plan (FR-SC-01). The profile and the agent are bound by composite
-- foreign keys inside the tenant, so a plan of one tenant cannot name the profile or the agent of
-- another (FR-TN-01). The name is unique for the tenant.
--
-- The time zone is an IANA name; the control plane validates it with the cron library before it
-- writes the row (FR-PF-05). The missed run policy is a closed set (FR-SC-07).

-- The composite key for the tenant-scoped profile foreign key below.
ALTER TABLE profiles ADD CONSTRAINT profiles_tenant_id_unique UNIQUE (tenant_id, id);

CREATE TABLE plans (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    -- The backup definition that the scheduler compiles for each job (FR-PF-04).
    profile_id uuid NOT NULL,
    -- The agent that runs the plan (FR-SC-11). One agent for the MVP.
    agent_id uuid NOT NULL,
    -- The schedule: a five-field cron expression with a time zone (FR-SC-01).
    cron text NOT NULL,
    -- An IANA time zone name, for example `Europe/Berlin` (D8).
    time_zone text NOT NULL,
    -- The missed run policy: run every missed fire time, the latest only, or none (FR-SC-07).
    missed_run_policy text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    -- The next fire time in UTC. Null until the control plane sets the first one (D8).
    next_run_at timestamptz,
    -- The last fire time that the scheduler processed: it wrote a job for it, or the missed run
    -- policy skipped it, in UTC.
    last_fire_time timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT plans_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT plans_tenant_id_unique UNIQUE (tenant_id, id),
    CONSTRAINT plans_tenant_name_unique UNIQUE (tenant_id, name),
    CONSTRAINT plans_profile_fk FOREIGN KEY (tenant_id, profile_id) REFERENCES profiles (tenant_id, id),
    CONSTRAINT plans_agent_fk FOREIGN KEY (tenant_id, agent_id) REFERENCES agents (tenant_id, id),
    CONSTRAINT plans_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id),
    CONSTRAINT plans_missed_run_policy_check CHECK (
        missed_run_policy IN ('run_all', 'latest_only', 'skip')
    )
);

CREATE INDEX plans_tenant_created_idx ON plans (tenant_id, created_at DESC, id DESC);

-- The scheduler claims the due plans of a tenant. The partial index keeps the scan to the enabled
-- plans that hold a due time (ADR-0010).
CREATE INDEX plans_due_idx ON plans (next_run_at) WHERE enabled AND next_run_at IS NOT NULL;

-- The account row lives in the platform tenant, so the creator is guarded by the same trigger as
-- the profiles and the jobs (ADR-0025, migration 0018).
CREATE TRIGGER plans_creator_membership
    BEFORE INSERT OR UPDATE ON plans
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

ALTER TABLE plans ENABLE ROW LEVEL SECURITY;
ALTER TABLE plans FORCE ROW LEVEL SECURITY;
CREATE POLICY plans_tenant ON plans
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON plans TO restichub_app;

-- The scheduled jobs. A job that the scheduler creates names the plan and the fire time
-- (FR-SC-02). The unique key makes the create idempotent: two control plane instances cannot
-- create the same job (ADR-0010).
ALTER TABLE jobs ADD COLUMN plan_id uuid;
ALTER TABLE jobs ADD COLUMN fire_time timestamptz;

-- A plan delete keeps the job history (FR-LG-03): the link becomes null and the job row stays.
ALTER TABLE jobs ADD CONSTRAINT jobs_plan_fk FOREIGN KEY (tenant_id, plan_id)
    REFERENCES plans (tenant_id, id) ON DELETE SET NULL (plan_id);

ALTER TABLE jobs ADD CONSTRAINT jobs_plan_fire_unique UNIQUE (plan_id, fire_time);

-- One active job for each repository, for the maintenance kinds only. Several backup jobs may run
-- on one repository at the same time (FR-SC-04), so the partial index covers `init`, `test_access`,
-- `forget`, `prune` and `check` only. The barrier that serializes a maintenance kind against the
-- others is part of the queue (a later part of epic #7).
DROP INDEX jobs_active_repository;
CREATE UNIQUE INDEX jobs_active_repository ON jobs (repository_id)
    WHERE repository_id IS NOT NULL
      AND kind IN ('init', 'test_access', 'forget', 'prune', 'check')
      AND status IN ('queued', 'sent', 'running');
