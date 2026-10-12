-- Tenant keys, the key-encryption-key check and the encrypted secrets
-- (ADR-0023, section Secrets; ADR-0017).
--
-- Each tenant has a data encryption key (DEK). The KEK wraps the DEK with AES-256-GCM. A secret
-- value uses AES-256-GCM with the DEK and the associated data `tenant_id || secret_id || purpose`.
-- The KEK check row holds encrypted known plaintext. It belongs to the platform tenant.

CREATE TABLE tenant_keys (
    tenant_id uuid NOT NULL,
    key_version integer NOT NULL,
    kek_id text NOT NULL,
    wrapped_dek bytea NOT NULL,
    nonce bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    active boolean NOT NULL DEFAULT true,
    CONSTRAINT tenant_keys_pkey PRIMARY KEY (tenant_id, key_version),
    CONSTRAINT tenant_keys_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id)
);

-- One active key for each tenant (D4).
CREATE UNIQUE INDEX tenant_keys_one_active ON tenant_keys (tenant_id) WHERE active;

CREATE TABLE kek_check (
    tenant_id uuid NOT NULL,
    kek_id text NOT NULL,
    nonce bytea NOT NULL,
    ciphertext bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT kek_check_pkey PRIMARY KEY (tenant_id, kek_id),
    CONSTRAINT kek_check_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT kek_check_platform_only
        CHECK (tenant_id = '00000000-0000-7000-8000-000000000000')
);

CREATE TABLE secrets (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    purpose text NOT NULL,
    key_version integer NOT NULL,
    nonce bytea NOT NULL,
    ciphertext bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_by uuid,
    CONSTRAINT secrets_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT secrets_tenant_name_unique UNIQUE (tenant_id, name),
    CONSTRAINT secrets_tenant_key_fk FOREIGN KEY (tenant_id, key_version)
        REFERENCES tenant_keys (tenant_id, key_version),
    CONSTRAINT secrets_created_by_fk FOREIGN KEY (tenant_id, created_by)
        REFERENCES users (tenant_id, id)
);

ALTER TABLE tenant_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_keys FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_keys_tenant ON tenant_keys
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE kek_check ENABLE ROW LEVEL SECURITY;
ALTER TABLE kek_check FORCE ROW LEVEL SECURITY;
CREATE POLICY kek_check_tenant ON kek_check
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE secrets ENABLE ROW LEVEL SECURITY;
ALTER TABLE secrets FORCE ROW LEVEL SECURITY;
CREATE POLICY secrets_tenant ON secrets
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);
