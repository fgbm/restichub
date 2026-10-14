-- The profiles (FR-PF-01, FR-PF-03, FR-HK-02, FR-HK-04, ADR-0004, ADR-0006).
--
-- One row per profile, under the tenant policy of ADR-0023. A profile is a declarative backup
-- definition (FR-PF-01): the repository, the paths for each operating system, the excludes, the
-- exclude files, the tags, the backup options, the hooks, the retention and the check options.
--
-- The repository is bound by a composite foreign key inside the tenant, so a profile of one tenant
-- cannot name the repository of another (FR-TN-01). The name is unique for the tenant.
--
-- The list settings use text arrays. The retention, the check options and the hooks use a jsonb
-- document. The document is a typed value, not a free-form argument map: the control plane
-- validates it with the compiler before it writes the row (FR-PF-05, FR-PR-01). The check
-- constraints below refuse a document of the wrong JSON kind.
--
-- The schedule is not here: it belongs to the plan (FR-SC-01). The cron expression is validated
-- with `restichub_compiler::validate_cron` when the plan is saved.

-- The composite key for the tenant-scoped repository foreign key below.
ALTER TABLE repositories ADD CONSTRAINT repositories_tenant_id_unique UNIQUE (tenant_id, id);

CREATE TABLE profiles (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    repository_id uuid NOT NULL,
    -- The paths of every operating system and the extra paths of one operating system (FR-PF-01).
    paths_all text[] NOT NULL DEFAULT '{}',
    paths_windows text[] NOT NULL DEFAULT '{}',
    paths_linux text[] NOT NULL DEFAULT '{}',
    paths_macos text[] NOT NULL DEFAULT '{}',
    excludes text[] NOT NULL DEFAULT '{}',
    exclude_files text[] NOT NULL DEFAULT '{}',
    tags text[] NOT NULL DEFAULT '{}',
    -- The backup options (FR-PF-01). The VSS flag is `use_fs_snapshot` (Windows only).
    use_fs_snapshot boolean NOT NULL DEFAULT false,
    one_file_system boolean NOT NULL DEFAULT false,
    exclude_caches boolean NOT NULL DEFAULT false,
    limit_upload_kib bigint NOT NULL DEFAULT 0,
    limit_download_kib bigint NOT NULL DEFAULT 0,
    -- The retention policy that produces the `forget` and `prune` steps (FR-PF-01).
    retention jsonb,
    -- The check options that produce the verification step (FR-PF-01).
    check_options jsonb,
    -- The hooks, by allow-list identifier, with their typed parameters (FR-HK-02, FR-HK-03).
    hooks jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT profiles_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT profiles_tenant_name_unique UNIQUE (tenant_id, name),
    CONSTRAINT profiles_repository_fk FOREIGN KEY (tenant_id, repository_id)
        REFERENCES repositories (tenant_id, id),
    CONSTRAINT profiles_created_by_fk FOREIGN KEY (created_by) REFERENCES users (id),
    CONSTRAINT profiles_limits_check CHECK (limit_upload_kib >= 0 AND limit_download_kib >= 0),
    CONSTRAINT profiles_retention_object_check CHECK (
        retention IS NULL OR jsonb_typeof(retention) = 'object'
    ),
    CONSTRAINT profiles_check_options_object_check CHECK (
        check_options IS NULL OR jsonb_typeof(check_options) = 'object'
    ),
    CONSTRAINT profiles_hooks_array_check CHECK (jsonb_typeof(hooks) = 'array')
);

CREATE INDEX profiles_tenant_created_idx ON profiles (tenant_id, created_at DESC, id DESC);

-- The account row lives in the platform tenant, so the creator is guarded by the same trigger as
-- the secrets and the repositories (ADR-0025, migration 0018).
CREATE TRIGGER profiles_creator_membership
    BEFORE INSERT OR UPDATE ON profiles
    FOR EACH ROW EXECUTE FUNCTION check_creator_membership();

ALTER TABLE profiles ENABLE ROW LEVEL SECURITY;
ALTER TABLE profiles FORCE ROW LEVEL SECURITY;
CREATE POLICY profiles_tenant ON profiles
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON profiles TO restichub_app;
