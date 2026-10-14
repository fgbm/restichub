-- The maintenance schedules of a repository (FR-RP-08, FR-SC-04, ADR-0034).
--
-- A repository has two schedules: the retention cycle (`forget` and `prune`) and the structure
-- check. The maintenance agent of the repository runs both. Each row holds a cron expression and an
-- IANA time zone, the same way a plan does (FR-SC-01). The scheduler reads the due rows with
-- `FOR UPDATE SKIP LOCKED`, so two control plane instances claim a row one at a time.
--
-- A scheduled maintenance job names the schedule and the fire time. The unique key makes the create
-- idempotent: two instances cannot create the same job (ADR-0010). The attempt is part of the key,
-- as it is for a plan, because a lock retry is a new row with the same fire time (ADR-0020).
--
-- The scheduler creates the default rows of a repository that has none (the repositories that exist
-- before this migration, for example), so the migration writes no data.

CREATE TABLE maintenance_schedules (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    repository_id uuid NOT NULL,
    -- The closed set of the maintenance tasks: `retention` runs `forget` and `prune`.
    task text NOT NULL,
    cron text NOT NULL,
    time_zone text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    -- The next fire time in UTC. Null until the scheduler or the API computes the first one.
    next_run_at timestamptz,
    last_fire_time timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT maintenance_schedules_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT maintenance_schedules_tenant_id_unique UNIQUE (tenant_id, id),
    -- A repository delete removes its schedules. The finished jobs keep their history.
    CONSTRAINT maintenance_schedules_repository_fk FOREIGN KEY (tenant_id, repository_id)
        REFERENCES repositories (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT maintenance_schedules_repository_task_unique UNIQUE (repository_id, task),
    CONSTRAINT maintenance_schedules_task_check CHECK (task IN ('retention', 'check'))
);

CREATE INDEX maintenance_schedules_due_idx ON maintenance_schedules (next_run_at)
    WHERE enabled;

ALTER TABLE maintenance_schedules ENABLE ROW LEVEL SECURITY;
ALTER TABLE maintenance_schedules FORCE ROW LEVEL SECURITY;
CREATE POLICY maintenance_schedules_tenant ON maintenance_schedules
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON maintenance_schedules TO restichub_app;

ALTER TABLE jobs ADD COLUMN maintenance_schedule_id uuid;

-- A schedule delete keeps the job history: the link becomes null and the job row stays.
ALTER TABLE jobs ADD CONSTRAINT jobs_maintenance_schedule_fk
    FOREIGN KEY (tenant_id, maintenance_schedule_id)
    REFERENCES maintenance_schedules (tenant_id, id) ON DELETE SET NULL (maintenance_schedule_id);

ALTER TABLE jobs ADD CONSTRAINT jobs_maintenance_fire_attempt_unique
    UNIQUE (maintenance_schedule_id, fire_time, attempt);
