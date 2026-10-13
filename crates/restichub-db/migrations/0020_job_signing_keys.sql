-- The Ed25519 job signing keys (ADR-0026, ADR-0005, ADR-0006).
--
-- One key set serves all tenants and lives in the platform tenant. The set holds the current key
-- that signs the jobs and the next key that the agents receive before a rotation. The key
-- encryption key (KEK) wraps each private key with the associated data
-- `restichub-signing-key-v1\0 || tenant_id || key_id || role` (ADR-0026, section CA key format).
-- The key ID in the associated data binds a blob to its row, and `public_key` binds the unwrapped
-- key to the row (T19).
--
-- A rotation promotes the next key to current, generates a new next key and marks the old current
-- key for retirement. A key with `retired_at` set is no longer used and its identifier is sent to
-- the agents as a revoked key. The overlap keeps the old key valid until the agents acknowledge the
-- new set or `retire_after` passes.

CREATE TABLE signing_keys (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    role text NOT NULL,
    kek_id text NOT NULL,
    nonce bytea NOT NULL,
    wrapped_key bytea NOT NULL,
    public_key bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    retire_after timestamptz,
    retired_at timestamptz,
    CONSTRAINT signing_keys_pkey PRIMARY KEY (tenant_id, id),
    CONSTRAINT signing_keys_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT signing_keys_role_check CHECK (role IN ('current', 'next')),
    CONSTRAINT signing_keys_platform_only
        CHECK (tenant_id = '00000000-0000-7000-8000-000000000000'),
    CONSTRAINT signing_keys_retire_check
        CHECK (retired_at IS NULL OR retire_after IS NOT NULL)
);

-- One active current key and one active next key. A retired row and a row that is scheduled for
-- retirement are outside the index, so a rotation keeps the overlap rows and adds the new active
-- ones (ADR-0026).
CREATE UNIQUE INDEX signing_keys_one_active_role
    ON signing_keys (tenant_id, role) WHERE retired_at IS NULL AND retire_after IS NULL;

ALTER TABLE signing_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE signing_keys FORCE ROW LEVEL SECURITY;
CREATE POLICY signing_keys_tenant ON signing_keys
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON signing_keys TO restichub_app;
