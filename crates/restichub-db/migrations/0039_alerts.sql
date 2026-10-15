-- The alerts, the notification channels and the recovery point objective (FR-MN-01, FR-MN-03,
-- FR-MN-04, AC-MN-01, AC-MN-02, ADR-0036, ADR-0006, ADR-0017).
--
-- `plans.rpo_seconds` holds the recovery point objective of a plan. Null means the default that
-- the control plane derives from the schedule (ADR-0036).
--
-- `alert_rules` holds the settings of the alert kinds of a tenant. A missing row means the default.
--
-- `notification_channels` holds the email and webhook targets. The signing secret of a webhook and
-- the path and the query of its URL are rows of `secrets`; they are never part of this table in
-- clear text (ADR-0017). The table keeps the scheme, the host and the port of the URL only.
--
-- `alert_state` is the idempotent state of the alerts. The partial unique index allows one open
-- row for each `(kind, dedupe_key)` pair, so a repeated evaluation cannot open a second alert.
--
-- `alert_deliveries` is the outbox. A state change and its deliveries commit in one transaction.
-- A sender claims a row with `FOR UPDATE SKIP LOCKED`, so two control plane instances send once.

ALTER TABLE plans ADD COLUMN rpo_seconds integer;
ALTER TABLE plans ADD CONSTRAINT plans_rpo_seconds_check CHECK (
    rpo_seconds IS NULL OR rpo_seconds BETWEEN 900 AND 34560000
);

CREATE TABLE alert_rules (
    tenant_id uuid NOT NULL,
    kind text NOT NULL,
    enabled boolean NOT NULL,
    -- Minutes. Set for the kinds `agent_offline` and `stale_lock` only.
    threshold_minutes integer,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, kind),
    CONSTRAINT alert_rules_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT alert_rules_kind_check CHECK (
        kind IN ('rpo_violated', 'job_failed', 'job_dead_letter', 'job_warnings', 'agent_offline',
                 'stale_lock')
    ),
    CONSTRAINT alert_rules_threshold_check CHECK (
        threshold_minutes IS NULL OR threshold_minutes BETWEEN 1 AND 525600
    )
);

CREATE TABLE notification_channels (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    kind text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    recipients text[] NOT NULL DEFAULT '{}',
    url_origin text,
    secret_id uuid,
    url_secret_id uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT notification_channels_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT notification_channels_tenant_id_unique UNIQUE (tenant_id, id),
    CONSTRAINT notification_channels_name_unique UNIQUE (tenant_id, name),
    CONSTRAINT notification_channels_secret_fk FOREIGN KEY (tenant_id, secret_id)
        REFERENCES secrets (tenant_id, id),
    CONSTRAINT notification_channels_url_secret_fk FOREIGN KEY (tenant_id, url_secret_id)
        REFERENCES secrets (tenant_id, id),
    CONSTRAINT notification_channels_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id),
    CONSTRAINT notification_channels_kind_check CHECK (
        (kind = 'email' AND cardinality(recipients) BETWEEN 1 AND 20
            AND url_origin IS NULL AND secret_id IS NULL AND url_secret_id IS NULL)
        OR (kind = 'webhook' AND cardinality(recipients) = 0
            AND url_origin IS NOT NULL AND secret_id IS NOT NULL AND url_secret_id IS NOT NULL)
    )
);

CREATE INDEX notification_channels_tenant_idx ON notification_channels (tenant_id, created_at, id);

CREATE TRIGGER notification_channels_creator_membership
    BEFORE INSERT OR UPDATE ON notification_channels
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

CREATE TABLE alert_state (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    kind text NOT NULL,
    dedupe_key text NOT NULL,
    status text NOT NULL,
    agent_id uuid,
    plan_id uuid,
    repository_id uuid,
    job_id uuid,
    -- Display values copied at the time of the alert, so the alert stays readable after a delete.
    agent_hostname text,
    plan_name text,
    repository_name text,
    -- The typed values of the condition: the numbers, the codes and the times. The interface and
    -- the messages build the text from them in the language of the reader.
    params jsonb NOT NULL DEFAULT '{}'::jsonb,
    -- The scrubbed and bounded error text of a job. Empty when the condition has none.
    detail text NOT NULL DEFAULT '',
    opened_at timestamptz NOT NULL,
    resolved_at timestamptz,
    CONSTRAINT alert_state_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT alert_state_tenant_id_unique UNIQUE (tenant_id, id),
    CONSTRAINT alert_state_agent_fk FOREIGN KEY (tenant_id, agent_id)
        REFERENCES agents (tenant_id, id) ON DELETE SET NULL (agent_id),
    CONSTRAINT alert_state_plan_fk FOREIGN KEY (tenant_id, plan_id)
        REFERENCES plans (tenant_id, id) ON DELETE SET NULL (plan_id),
    CONSTRAINT alert_state_repository_fk FOREIGN KEY (tenant_id, repository_id)
        REFERENCES repositories (tenant_id, id) ON DELETE SET NULL (repository_id),
    CONSTRAINT alert_state_job_fk FOREIGN KEY (tenant_id, job_id)
        REFERENCES jobs (tenant_id, id) ON DELETE SET NULL (job_id),
    CONSTRAINT alert_state_kind_check CHECK (
        kind IN ('rpo_violated', 'job_failed', 'job_dead_letter', 'job_warnings', 'agent_offline',
                 'stale_lock')
    ),
    CONSTRAINT alert_state_status_check CHECK (
        (status = 'open' AND resolved_at IS NULL)
        OR (status = 'resolved' AND resolved_at IS NOT NULL)
    ),
    CONSTRAINT alert_state_detail_check CHECK (char_length(detail) <= 1000)
);

CREATE UNIQUE INDEX alert_state_open_unique ON alert_state (tenant_id, kind, dedupe_key)
    WHERE status = 'open';
CREATE INDEX alert_state_list_idx ON alert_state (tenant_id, opened_at DESC, id DESC);
CREATE INDEX alert_state_resolved_idx ON alert_state (resolved_at) WHERE status = 'resolved';

CREATE TABLE alert_deliveries (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    alert_id uuid NOT NULL,
    channel_id uuid NOT NULL,
    event text NOT NULL,
    status text NOT NULL DEFAULT 'pending',
    attempts integer NOT NULL DEFAULT 0,
    -- The earliest time of the next attempt. A claim moves it forward, so the claim is a lease.
    next_attempt_at timestamptz NOT NULL,
    last_error text,
    created_at timestamptz NOT NULL,
    sent_at timestamptz,
    CONSTRAINT alert_deliveries_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT alert_deliveries_alert_fk FOREIGN KEY (tenant_id, alert_id)
        REFERENCES alert_state (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT alert_deliveries_channel_fk FOREIGN KEY (tenant_id, channel_id)
        REFERENCES notification_channels (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT alert_deliveries_event_check CHECK (event IN ('opened', 'resolved')),
    CONSTRAINT alert_deliveries_status_check CHECK (status IN ('pending', 'sent', 'failed')),
    CONSTRAINT alert_deliveries_error_check CHECK (last_error IS NULL OR char_length(last_error) <= 300)
);

CREATE INDEX alert_deliveries_due_idx ON alert_deliveries (tenant_id, next_attempt_at)
    WHERE status = 'pending';
CREATE INDEX alert_deliveries_created_idx ON alert_deliveries (created_at) WHERE status <> 'pending';

-- The RPO evaluation reads the newest succeeded backup of each plan.
CREATE INDEX jobs_plan_last_success_idx ON jobs (tenant_id, plan_id, finished_at DESC)
    WHERE kind = 'backup' AND status = 'succeeded';
-- The alert evaluation reads the newest finished job of each subject inside a time window.
CREATE INDEX jobs_finished_window_idx
    ON jobs (tenant_id, (COALESCE(finished_at, updated_at)) DESC)
    WHERE status IN ('succeeded', 'failed', 'dead_letter');

ALTER TABLE alert_rules ENABLE ROW LEVEL SECURITY;
ALTER TABLE alert_rules FORCE ROW LEVEL SECURITY;
CREATE POLICY alert_rules_tenant ON alert_rules
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE notification_channels ENABLE ROW LEVEL SECURITY;
ALTER TABLE notification_channels FORCE ROW LEVEL SECURITY;
CREATE POLICY notification_channels_tenant ON notification_channels
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE alert_state ENABLE ROW LEVEL SECURITY;
ALTER TABLE alert_state FORCE ROW LEVEL SECURITY;
CREATE POLICY alert_state_tenant ON alert_state
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE alert_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE alert_deliveries FORCE ROW LEVEL SECURITY;
CREATE POLICY alert_deliveries_tenant ON alert_deliveries
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON alert_rules TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON notification_channels TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON alert_state TO restichub_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON alert_deliveries TO restichub_app;
