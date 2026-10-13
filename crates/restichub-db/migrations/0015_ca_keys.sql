-- The certificate authority private keys and certificates (ADR-0026, ADR-0006).
--
-- The platform tenant holds one root CA and one or more issuing CAs. The key encryption key (KEK)
-- wraps each private key with the associated data `restichub-ca-key-v1\0 || tenant_id || role`
-- (ADR-0026, section CA key format). The certificates are stored in clear, because a certificate
-- is public. The control plane stores no agent private key.
--
-- The issuing CA rotates with an overlap: the root signs a new issuing CA and both stay valid
-- until the older certificate expires, so more than one issuing row is present at that time.

CREATE TABLE ca_keys (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    role text NOT NULL,
    kek_id text NOT NULL,
    nonce bytea NOT NULL,
    wrapped_key bytea NOT NULL,
    cert_der bytea NOT NULL,
    not_before timestamptz NOT NULL,
    not_after timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT ca_keys_pkey PRIMARY KEY (tenant_id, id),
    CONSTRAINT ca_keys_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT ca_keys_role_check CHECK (role IN ('root', 'issuing')),
    CONSTRAINT ca_keys_platform_only
        CHECK (tenant_id = '00000000-0000-7000-8000-000000000000')
);

-- One root CA for the installation. The issuing CA has no such index: a rotation adds a second row.
CREATE UNIQUE INDEX ca_keys_one_root ON ca_keys (tenant_id) WHERE role = 'root';

ALTER TABLE ca_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE ca_keys FORCE ROW LEVEL SECURITY;
CREATE POLICY ca_keys_tenant ON ca_keys
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT ON ca_keys TO restichub_app;
