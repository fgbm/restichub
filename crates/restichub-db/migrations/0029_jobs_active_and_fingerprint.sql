-- One active job for each repository, and the location fingerprint of a job.
--
-- One active job for each repository: a second job while a job is queued, sent or running is
-- refused with HTTP 409. The partial index carries the invariant, so two control plane instances
-- cannot both start a job for one repository.
--
-- The job row stores a fingerprint of the repository location at create time. The control plane
-- applies the `initialized` flag only while the fingerprint still matches, so a location change
-- between the job and its result cannot mark the wrong repository ready. The fingerprint is a hash
-- of the stored location and, for a local backend, of the maintenance agent; it holds no secret.
--
-- The result code gains `access_error`: a `not_initialized` result on an initialized repository is
-- a location mismatch, not a normal result.

CREATE UNIQUE INDEX jobs_active_repository ON jobs (repository_id)
    WHERE repository_id IS NOT NULL AND status IN ('queued', 'sent', 'running');

ALTER TABLE jobs ADD COLUMN location_fingerprint text;

ALTER TABLE jobs DROP CONSTRAINT jobs_result_code_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_result_code_check CHECK (
    result_code IS NULL
    OR result_code IN ('succeeded', 'failed', 'config_error', 'not_initialized', 'cancelled', 'access_error')
);
