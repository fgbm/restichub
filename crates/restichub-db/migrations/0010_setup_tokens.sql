-- The one-time setup tokens (issue #18, FR-AU-03).
--
-- The server generates a setup token at start when the platform tenant has no user. The database
-- keeps only the SHA-256 hash of the token. The token has an expiry of 24 hours. The table uses
-- the general tenant policy. The tenant of a setup token is the platform tenant.
--
-- The runtime role gets SELECT, INSERT and UPDATE. A setup token is marked used, never removed.

CREATE TABLE setup_tokens (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    token_hash bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    CONSTRAINT setup_tokens_token_hash_unique UNIQUE (token_hash),
    CONSTRAINT setup_tokens_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id)
);

ALTER TABLE setup_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE setup_tokens FORCE ROW LEVEL SECURITY;
CREATE POLICY setup_tokens_tenant ON setup_tokens
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON setup_tokens TO restichub_app;
