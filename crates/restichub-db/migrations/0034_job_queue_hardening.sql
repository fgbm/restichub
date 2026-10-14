-- The hardening of the job queue (FR-SC-03, FR-SC-05, FR-SC-06, FR-PR-06, ADR-0010, ADR-0017,
-- ADR-0020).
--
-- A lock retry is a new row (ADR-0020): the agent journal returns the stored result for a known
-- `job_id`, so a second run needs a second identifier. `retry_of` links the new row to the row that
-- reported the lock, and the unique key of a scheduled job includes the attempt.
--
-- `lock_reported` is set by a step result with the retry status (restic exit code 11). The agent
-- reports the lock on the step, and the job result of a stopped job has the status `failed`.
--
-- `cancel_requested` is set by a cancel of a sent or running job. The dispatcher never sends a
-- flagged job, and a late acknowledgement of a flagged job gets a cancel message.
--
-- The `signed_job_*` columns hold the signed job of a `sent` row, encrypted with the data
-- encryption key of the tenant. A redelivery sends the same signed bytes (ADR-0020). The trigger
-- drops the ciphertext when the row reaches a terminal status or starts to run: the agent holds
-- the job from its acknowledgement, so the control plane never sends it again.
-- `signed_job_secrets` lists the secrets that the job carries, so each delivery writes its
-- `secret_delivered` records (AC-RP-03).
--
-- `timeout_seconds` is the job timeout of the signed job. A `running` job that outlives its timeout
-- and a grace period fails with the reason `timed_out`, so a restart of the agent never holds a
-- slot for ever (FR-SC-06).

ALTER TABLE jobs ADD COLUMN cancel_requested boolean NOT NULL DEFAULT false;
ALTER TABLE jobs ADD COLUMN lock_reported boolean NOT NULL DEFAULT false;
ALTER TABLE jobs ADD COLUMN retry_of uuid;
ALTER TABLE jobs ADD COLUMN timeout_seconds integer;
ALTER TABLE jobs ADD COLUMN signed_job bytea;
ALTER TABLE jobs ADD COLUMN signed_job_nonce bytea;
ALTER TABLE jobs ADD COLUMN signed_job_key_version integer;
ALTER TABLE jobs ADD COLUMN signed_job_secrets uuid[] NOT NULL DEFAULT '{}';

ALTER TABLE jobs DROP CONSTRAINT jobs_plan_fire_unique;
ALTER TABLE jobs ADD CONSTRAINT jobs_plan_fire_attempt_unique UNIQUE (plan_id, fire_time, attempt);

-- A plan has at most one queued manual run. A retry of a manual run has `attempt > 0`, so it never
-- conflicts with a later manual run.
CREATE UNIQUE INDEX jobs_manual_queued_plan ON jobs (plan_id)
    WHERE plan_id IS NOT NULL AND fire_time IS NULL AND kind = 'backup'
      AND status = 'queued' AND attempt = 0;

CREATE FUNCTION jobs_drop_signed_job() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    NEW.signed_job := NULL;
    NEW.signed_job_nonce := NULL;
    NEW.signed_job_key_version := NULL;
    RETURN NEW;
END
$$;

CREATE TRIGGER jobs_drop_signed_job
    BEFORE UPDATE ON jobs
    FOR EACH ROW
    WHEN (NEW.status IN ('succeeded', 'failed', 'expired', 'dead_letter', 'queued', 'running'))
    EXECUTE FUNCTION jobs_drop_signed_job();
