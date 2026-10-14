-- The persistent job queue (FR-SC-03, FR-SC-04, FR-SC-05, FR-SC-06, FR-PR-06, ADR-0010).
--
-- The `jobs` table is the queue. The dispatcher of the control plane selects the dispatchable rows
-- with `FOR UPDATE SKIP LOCKED`, so two instances never send the same job twice.
--
-- `run_at` is the earliest time at which the dispatcher may send a job that a lock retry delayed
-- (restic exit code 11); the delay grows exponentially. A job without a retry runs at once. `lease_until` is the time at
-- which a `sent` job without an acknowledgement is delivered again (FR-PR-06). `attempt` counts the
-- lock retries; `deliveries` counts the sends of the job. `stale_lock_since` marks a repository
-- lock that may be stale (FR-SC-06): the time when the agent was lost, or the time of a hard kill.
--
-- The status `dead_letter` is terminal: the job exhausted its lock retries (FR-SC-05). The kind
-- `unlock_all` is the Admin action that removes every lock (FR-SC-06).

ALTER TABLE jobs ADD COLUMN run_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE jobs ADD COLUMN lease_until timestamptz;
ALTER TABLE jobs ADD COLUMN attempt integer NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN deliveries integer NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN stale_lock_since timestamptz;

ALTER TABLE jobs DROP CONSTRAINT jobs_kind_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_kind_check CHECK (
    kind IN ('init', 'test_access', 'backup', 'check', 'forget', 'prune', 'restore', 'unlock',
             'unlock_all', 'other')
);

ALTER TABLE jobs DROP CONSTRAINT jobs_status_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_status_check CHECK (
    status IN ('queued', 'sent', 'running', 'succeeded', 'failed', 'expired', 'dead_letter')
);

-- The dispatcher scans the waiting rows of a tenant in the order of creation.
CREATE INDEX jobs_waiting_idx ON jobs (tenant_id, created_at, id)
    WHERE status IN ('queued', 'sent');

-- The limits count the sent and running rows of a tenant.
CREATE INDEX jobs_in_flight_idx ON jobs (tenant_id, agent_id)
    WHERE status IN ('sent', 'running');

-- The stale lock recovery scans the marked rows only.
CREATE INDEX jobs_stale_lock_idx ON jobs (tenant_id, stale_lock_since)
    WHERE stale_lock_since IS NOT NULL;
