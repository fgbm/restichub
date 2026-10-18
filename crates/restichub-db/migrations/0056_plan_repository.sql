-- The repository of a plan (ADR-0047, FR-SC-01, FR-PF-01).
--
-- A profile is a declarative backup definition and says what to back up and how. A plan binds a
-- profile to one agent, one repository and one schedule. The repository leaves the profile, so one
-- profile can back up into several repositories and one repository can receive several profiles.
--
-- Each plan keeps the repository of its former profile. The old column was bound by a composite
-- foreign key inside the tenant (FR-TN-01), so the copied value stays inside the tenant and the
-- new key repeats the same rule.
--
-- The maintenance job reads the plans of one repository, and the delete of a repository asks which
-- plans hold it, so the new key has an index (FR-RP-08).

ALTER TABLE plans ADD COLUMN repository_id uuid;

UPDATE plans p
SET repository_id = pr.repository_id
FROM profiles pr
WHERE pr.tenant_id = p.tenant_id AND pr.id = p.profile_id;

ALTER TABLE plans ALTER COLUMN repository_id SET NOT NULL;

ALTER TABLE plans ADD CONSTRAINT plans_repository_fk FOREIGN KEY (tenant_id, repository_id)
    REFERENCES repositories (tenant_id, id);

CREATE INDEX plans_tenant_repository_idx ON plans (tenant_id, repository_id);

ALTER TABLE profiles DROP CONSTRAINT profiles_repository_fk;

ALTER TABLE profiles DROP COLUMN repository_id;
