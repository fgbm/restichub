-- Restore and the browsing of snapshots (FR-RS-01 to FR-RS-04, ADR-0035).
--
-- The kinds `snapshots` and `ls` are on-demand read jobs that fill the snapshot list and the tree
-- of the Restore screen. A browse job and a restore never run alone on a repository: restic reads
-- a repository in parallel, and the queue barrier of an exclusive kind (prune, check) already keeps
-- them apart from the maintenance (FR-SC-04). The partial unique index of the active jobs of a
-- repository therefore excludes them, like `backup`.
--
-- `job_requests` holds the typed parameters of a browse job or a restore. The job row holds no
-- command (ADR-0010): the queue builds the steps from this row at the first send. `job_listings`
-- holds the result of a browse job, because the journal copy of the step result drops the large
-- outputs. Only the control plane writes a listing, after it bounded and cleaned each value that
-- came from the agent. The sweep of the control plane removes an old listing.
--
-- `job_steps` gets the summary of a restore (FR-RS-04).
--
-- The agent reports its restore roots in `AgentHello` (FR-RS-03, ADR-0035). The control plane keeps
-- the last report for the Restore screen, which suggests a root as the target. The live connection
-- is the authority for the early refusal; the agent checks the target again.

ALTER TABLE agents ADD COLUMN restore_roots text[] NOT NULL DEFAULT '{}';

ALTER TABLE jobs DROP CONSTRAINT jobs_kind_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_kind_check CHECK (
    kind IN ('init', 'test_access', 'backup', 'check', 'forget', 'prune', 'restore', 'unlock',
             'unlock_all', 'snapshots', 'ls', 'other')
);

DROP INDEX jobs_active_repository;
CREATE UNIQUE INDEX jobs_active_repository ON jobs (repository_id)
    WHERE repository_id IS NOT NULL
      AND kind NOT IN ('backup', 'snapshots', 'ls', 'restore')
      AND status IN ('queued', 'sent', 'running');

ALTER TABLE job_steps DROP CONSTRAINT job_steps_output_kind_check;
ALTER TABLE job_steps ADD CONSTRAINT job_steps_output_kind_check CHECK (
    output_kind IN ('none', 'backup', 'snapshots', 'forget', 'check', 'stats', 'ls', 'restore')
);
ALTER TABLE job_steps DROP CONSTRAINT job_steps_summary_check;
ALTER TABLE job_steps ADD CONSTRAINT job_steps_summary_check CHECK (
    (output_kind IN ('snapshots', 'forget', 'check', 'stats', 'restore')) = (summary IS NOT NULL)
);

CREATE TABLE job_requests (
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    request jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, job_id),
    CONSTRAINT job_requests_tenant_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT job_requests_job_fk FOREIGN KEY (tenant_id, job_id)
        REFERENCES jobs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT job_requests_object_check CHECK (jsonb_typeof(request) = 'object'),
    CONSTRAINT job_requests_size_check CHECK (pg_column_size(request) <= 65536)
);

CREATE TABLE job_listings (
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    kind text NOT NULL,
    payload jsonb NOT NULL,
    created_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, job_id),
    CONSTRAINT job_listings_tenant_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT job_listings_job_fk FOREIGN KEY (tenant_id, job_id)
        REFERENCES jobs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT job_listings_kind_check CHECK (kind IN ('snapshots', 'ls')),
    CONSTRAINT job_listings_object_check CHECK (jsonb_typeof(payload) = 'object')
);

CREATE INDEX job_listings_created_idx ON job_listings (tenant_id, kind, created_at);

ALTER TABLE job_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_requests FORCE ROW LEVEL SECURITY;
CREATE POLICY job_requests_tenant ON job_requests
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE job_listings ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_listings FORCE ROW LEVEL SECURITY;
CREATE POLICY job_listings_tenant ON job_listings
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON job_requests TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON job_listings TO restichub_app;
