-- The enrollment tokens (ADR-0026, ADR-0032, FR-ID-01, FR-ID-05, FR-TN-03).
--
-- One token per row, under the tenant policy of ADR-0023. The control plane stores only the SHA-256
-- hash and a short display prefix (ADR-0023, section API tokens). The value appears once, at the
-- create. A token is single use and it expires. The exchange that spends the token is the
-- enrollment endpoint (#48).
--
-- A revoke is a soft change: it sets `revoked_at` and keeps the row, so the audit trail and the
-- status stay. `consume` refuses a used, an expired and a revoked token. The runtime role has no
-- DELETE on this table (T17).

CREATE TABLE enrollment_tokens (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    token_hash bytea NOT NULL,
    display_prefix text NOT NULL,
    labels text[] NOT NULL DEFAULT '{}',
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    revoked_at timestamptz,
    created_by_user_id uuid NOT NULL,
    revoked_by_user_id uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT enrollment_tokens_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT enrollment_tokens_created_by_fk FOREIGN KEY (created_by_user_id) REFERENCES users (id),
    CONSTRAINT enrollment_tokens_revoked_by_fk FOREIGN KEY (revoked_by_user_id) REFERENCES users (id)
);

-- The hash is unique in the installation, so a lookup by hash returns one row.
CREATE UNIQUE INDEX enrollment_tokens_token_hash_unique ON enrollment_tokens (token_hash);

-- Keyset pagination orders by the creation time and the identifier.
CREATE INDEX enrollment_tokens_tenant_created_idx
    ON enrollment_tokens (tenant_id, created_at DESC, id DESC);

ALTER TABLE enrollment_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE enrollment_tokens FORCE ROW LEVEL SECURITY;
CREATE POLICY enrollment_tokens_tenant ON enrollment_tokens
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON enrollment_tokens TO restichub_app;
