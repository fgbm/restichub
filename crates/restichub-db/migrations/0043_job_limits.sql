-- The time limits of a job (ADR-0039).
--
-- A profile holds the stall timeout (minutes without progress after which the agent stops a step)
-- and the maximum duration (default 48 hours, zero means none).
--
-- `jobs.timeout_seconds` is the optional maximum duration of the signed job. The old fixed value
-- of 12 hours is not a limit any more, so a queued or running job that carries it loses it.

ALTER TABLE profiles ADD COLUMN stall_timeout_minutes integer NOT NULL DEFAULT 60;
ALTER TABLE profiles ADD COLUMN max_duration_minutes integer NOT NULL DEFAULT 2880;
UPDATE profiles SET max_duration_minutes = 2880;
ALTER TABLE profiles ADD CONSTRAINT profiles_job_limits_check CHECK (
    stall_timeout_minutes BETWEEN 5 AND 10080 AND max_duration_minutes BETWEEN 0 AND 43200
);

UPDATE jobs SET timeout_seconds = NULL
    WHERE timeout_seconds = 43200 AND status IN ('queued', 'sent', 'running');

-- The stall timeout of the signed job, for the job detail. NULL for a job that was sent before
-- the migration or by an older control plane.
ALTER TABLE jobs ADD COLUMN stall_timeout_seconds integer;
