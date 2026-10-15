-- The time limits of a job (ADR-0039).
--
-- A profile holds the stall timeout (minutes without progress after which the agent stops a step)
-- and the maximum duration (default 48 hours, zero means none).
--
-- `jobs.timeout_seconds` is the optional maximum duration of the signed job. A queued job is signed
-- again when it is sent, so it loses the fixed value of 12 hours. A job that was sent already
-- carries the value in its signature, and the control plane keeps it as its safety net.

ALTER TABLE profiles ADD COLUMN stall_timeout_minutes integer NOT NULL DEFAULT 60;
ALTER TABLE profiles ADD COLUMN max_duration_minutes integer NOT NULL DEFAULT 2880;
ALTER TABLE profiles ADD CONSTRAINT profiles_job_limits_check CHECK (
    stall_timeout_minutes BETWEEN 5 AND 10080 AND max_duration_minutes BETWEEN 0 AND 43200
);

UPDATE jobs SET timeout_seconds = NULL
    WHERE timeout_seconds = 43200 AND status = 'queued';

-- The stall timeout of the signed job, for the job detail. NULL for a job that was sent before
-- the migration or by an older control plane.
ALTER TABLE jobs ADD COLUMN stall_timeout_seconds integer;
