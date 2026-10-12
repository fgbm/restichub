-- Sessions and API tokens (ADR-0023, sections Sessions and API tokens).
--
-- The database keeps only the SHA-256 hash of a session token and of an API token.
-- The role `platform_admin` of an API token is valid in the platform tenant only (D2).

CREATE TABLE sessions (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    user_id uuid NOT NULL,
    token_hash bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    ip inet,
    user_agent text,
    CONSTRAINT sessions_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT sessions_token_hash_unique UNIQUE (token_hash),
    CONSTRAINT sessions_user_fk FOREIGN KEY (tenant_id, user_id)
        REFERENCES users (tenant_id, id)
);

CREATE TABLE api_tokens (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    token_hash bytea NOT NULL,
    display_prefix text NOT NULL,
    role text NOT NULL,
    created_by uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    last_used_at timestamptz,
    revoked_at timestamptz,
    CONSTRAINT api_tokens_role_check
        CHECK (role IN ('platform_admin', 'admin', 'operator', 'viewer')),
    CONSTRAINT api_tokens_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT api_tokens_token_hash_unique UNIQUE (token_hash),
    CONSTRAINT api_tokens_created_by_fk FOREIGN KEY (tenant_id, created_by)
        REFERENCES users (tenant_id, id),
    CONSTRAINT api_tokens_platform_admin_only_platform
        CHECK (
            role <> 'platform_admin'
            OR tenant_id = '00000000-0000-7000-8000-000000000000'
        )
);

ALTER TABLE sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE sessions FORCE ROW LEVEL SECURITY;
CREATE POLICY sessions_tenant ON sessions
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

ALTER TABLE api_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_tokens FORCE ROW LEVEL SECURITY;
CREATE POLICY api_tokens_tenant ON api_tokens
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);
