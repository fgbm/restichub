-- The agent registry (ADR-0026, FR-ID-04, FR-TN-01, FR-TN-03).
--
-- One row per enrolled agent, under the tenant policy of ADR-0023. The identity of an agent is the
-- SAN URI `restichub://tenant/<tenant_uuid>/agent/<agent_uuid>` (ADR-0026). The stream layer checks
-- the status and the certificate serial at each handshake and each stream open (T6, T17).
--
-- A revoke sets the status to `revoked` and keeps the row, so the stream layer still finds the row
-- and refuses the agent. The row records the actor of the revoke.

CREATE TABLE agents (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    hostname text NOT NULL,
    os text NOT NULL,
    arch text NOT NULL,
    agent_version text NOT NULL,
    certificate_serial bytea NOT NULL,
    certificate_expires_at timestamptz NOT NULL,
    status text NOT NULL,
    last_seen_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    revoked_by_user_id uuid,
    CONSTRAINT agents_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT agents_status_check CHECK (status IN ('active', 'revoked')),
    CONSTRAINT agents_revoke_check CHECK (
        (status = 'active' AND revoked_at IS NULL)
        OR (status = 'revoked' AND revoked_at IS NOT NULL)
    ),
    CONSTRAINT agents_certificate_serial_unique UNIQUE (certificate_serial),
    CONSTRAINT agents_revoked_by_fk FOREIGN KEY (revoked_by_user_id) REFERENCES users (id)
);

-- Keyset pagination orders by the creation time and the identifier.
CREATE INDEX agents_tenant_created_idx ON agents (tenant_id, created_at DESC, id DESC);

ALTER TABLE agents ENABLE ROW LEVEL SECURITY;
ALTER TABLE agents FORCE ROW LEVEL SECURITY;
CREATE POLICY agents_tenant ON agents
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON agents TO restichub_app;
