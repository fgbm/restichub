-- Job steps, job logs and the last progress of a running job (FR-MN-01, FR-MN-02, FR-LG-01,
-- FR-LG-03, ADR-0015).
--
-- Job metadata stays in the database for good (FR-LG-03). Only the log text ages out: the retention
-- sweep deletes `job_logs` rows and leaves every other row (AC-LG-01).
--
-- `job_steps` holds one typed result for each step of a job. The summary of a backup has typed
-- columns because the metrics and the user interface read them (FR-MN-02). The other summaries are
-- a small jsonb value that the control plane parses into a typed struct.
--
-- `job_logs` holds one scrubbed line for each row, append-only. `seq` is the position of the line in
-- the log of the job, starting at 1. `job_log_state` counts the stored bytes, so a job cannot grow
-- past the bound, and keeps the highest agent chunk number of each stream, so a log tail that the
-- agent sends again stores no line twice.

-- The composite foreign keys below need a key on (tenant_id, id). The primary key is the id alone.
ALTER TABLE jobs ADD CONSTRAINT jobs_tenant_id_unique UNIQUE (tenant_id, id);

CREATE TABLE job_steps (
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    step_id text NOT NULL,
    -- The order in which the steps first arrived.
    position bigint GENERATED ALWAYS AS IDENTITY,
    status text NOT NULL,
    exit_code integer NOT NULL,
    started_at timestamptz,
    finished_at timestamptz,
    -- A machine-readable error code and its parameters (NFR-DOC-04). Never a secret.
    error_code text,
    error_params jsonb,
    possible_stale_lock boolean NOT NULL DEFAULT false,
    output_kind text NOT NULL DEFAULT 'none',
    -- The summary of a backup step (FR-MN-02).
    files_new bigint,
    files_changed bigint,
    files_unmodified bigint,
    dirs_new bigint,
    dirs_changed bigint,
    dirs_unmodified bigint,
    data_blobs bigint,
    tree_blobs bigint,
    data_added bigint,
    data_added_packed bigint,
    total_files_processed bigint,
    total_bytes_processed bigint,
    total_duration_seconds double precision,
    snapshot_id text,
    dry_run boolean,
    -- The summary of a check, a snapshot list, a forget and a stats step.
    summary jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, job_id, step_id),
    CONSTRAINT job_steps_job_fk FOREIGN KEY (tenant_id, job_id)
        REFERENCES jobs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT job_steps_status_check CHECK (
        status IN ('success', 'warning', 'retry', 'config_error', 'cancelled', 'failed', 'skipped')
    ),
    CONSTRAINT job_steps_output_kind_check CHECK (
        output_kind IN ('none', 'backup', 'snapshots', 'forget', 'check', 'stats', 'ls')
    ),
    CONSTRAINT job_steps_step_id_check CHECK (char_length(step_id) BETWEEN 1 AND 64),
    CONSTRAINT job_steps_error_code_check CHECK (error_code IS NULL OR char_length(error_code) <= 64),
    CONSTRAINT job_steps_snapshot_id_check CHECK (snapshot_id IS NULL OR char_length(snapshot_id) <= 64),
    -- The backup columns are set exactly for a backup output.
    CONSTRAINT job_steps_backup_columns_check CHECK (
        (output_kind = 'backup') = (files_new IS NOT NULL AND data_added IS NOT NULL)
    ),
    CONSTRAINT job_steps_summary_check CHECK (
        (output_kind IN ('snapshots', 'forget', 'check', 'stats')) = (summary IS NOT NULL)
    )
);

CREATE INDEX job_steps_position_idx ON job_steps (tenant_id, job_id, position);

CREATE TABLE job_logs (
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    seq bigint NOT NULL,
    step_id text NOT NULL DEFAULT '',
    stream text NOT NULL,
    level text NOT NULL,
    -- The time at which the control plane stored the line. The agent sends no line time.
    ts timestamptz NOT NULL,
    text text NOT NULL,
    PRIMARY KEY (tenant_id, job_id, seq),
    CONSTRAINT job_logs_job_fk FOREIGN KEY (tenant_id, job_id)
        REFERENCES jobs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT job_logs_stream_check CHECK (stream IN ('stdout', 'stderr', 'agent')),
    CONSTRAINT job_logs_level_check CHECK (level IN ('info', 'warn', 'error')),
    CONSTRAINT job_logs_text_check CHECK (char_length(text) <= 8192),
    CONSTRAINT job_logs_step_id_check CHECK (char_length(step_id) <= 64)
);

-- The retention sweep deletes by age.
CREATE INDEX job_logs_ts_idx ON job_logs (tenant_id, ts);

CREATE TABLE job_log_state (
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    -- The number of the last stored line.
    last_seq bigint NOT NULL DEFAULT 0,
    -- The stored bytes of text. The write path refuses lines beyond the bound.
    bytes bigint NOT NULL DEFAULT 0,
    -- True after the write path refused a line.
    truncated boolean NOT NULL DEFAULT false,
    -- The highest chunk number of each stream that the control plane stored.
    stdout_chunk bigint NOT NULL DEFAULT 0,
    stderr_chunk bigint NOT NULL DEFAULT 0,
    agent_chunk bigint NOT NULL DEFAULT 0,
    -- Set when the retention sweep deleted the lines.
    purged_at timestamptz,
    PRIMARY KEY (tenant_id, job_id),
    CONSTRAINT job_log_state_job_fk FOREIGN KEY (tenant_id, job_id)
        REFERENCES jobs (tenant_id, id) ON DELETE CASCADE
);

CREATE TABLE job_progress (
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    step_id text NOT NULL,
    bytes_done bigint NOT NULL,
    total_bytes bigint NOT NULL,
    files_done bigint NOT NULL,
    total_files bigint NOT NULL,
    percent_done double precision NOT NULL,
    seconds_elapsed double precision NOT NULL,
    seconds_remaining bigint NOT NULL,
    current_files text[] NOT NULL DEFAULT '{}',
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, job_id),
    CONSTRAINT job_progress_job_fk FOREIGN KEY (tenant_id, job_id)
        REFERENCES jobs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT job_progress_percent_check CHECK (percent_done >= 0 AND percent_done <= 1),
    CONSTRAINT job_progress_step_id_check CHECK (char_length(step_id) BETWEEN 1 AND 64)
);

ALTER TABLE job_steps ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_steps FORCE ROW LEVEL SECURITY;
CREATE POLICY job_steps_tenant ON job_steps
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE job_logs ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_logs FORCE ROW LEVEL SECURITY;
CREATE POLICY job_logs_tenant ON job_logs
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE job_log_state ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_log_state FORCE ROW LEVEL SECURITY;
CREATE POLICY job_log_state_tenant ON job_log_state
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE job_progress ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_progress FORCE ROW LEVEL SECURITY;
CREATE POLICY job_progress_tenant ON job_progress
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON job_steps TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON job_logs TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON job_log_state TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON job_progress TO restichub_app;
