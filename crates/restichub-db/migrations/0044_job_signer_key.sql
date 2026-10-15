-- The identifier of the job signing key that signed the job, for the job detail. The signed bytes
-- are dropped when the job ends, so the identifier is a plain column. NULL for a queued job and for
-- a job that was sent before this migration.
ALTER TABLE jobs ADD COLUMN signer_key_id text;
