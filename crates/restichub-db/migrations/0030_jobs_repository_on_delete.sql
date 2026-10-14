-- The repository delete keeps the job history (FR-LG-03).
--
-- The job metadata stays in the database after the repository goes: the link becomes null and the
-- job row stays, so the job history and the audit record of the delete keep the story. The partial
-- unique index `jobs_active_repository` ignores a null repository_id, and an active job still
-- blocks the delete in the control plane.

ALTER TABLE jobs DROP CONSTRAINT jobs_repository_fk;
ALTER TABLE jobs ADD CONSTRAINT jobs_repository_fk FOREIGN KEY (tenant_id, repository_id)
    REFERENCES repositories (tenant_id, id) ON DELETE SET NULL (repository_id);
