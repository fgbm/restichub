-- The audit log (ADR-0023, section Audit log; FR-AU-01).
--
-- The table is append-only. A trigger rejects UPDATE, DELETE and TRUNCATE. The runtime role has
-- SELECT and INSERT only (migration 0007).
--
-- The platform scope reads all rows. A platform write sets the tenant id explicitly. The platform
-- transaction sets the platform scope and then calls `enter_tenant` (D3). The tenant id must be a
-- valid tenant (the foreign key).
--
-- The platform scope reaches two tables only: `tenants` (full, migration 0002) and `audit_log`
-- (read and insert, below). The users and the memberships of the platform tenant use a tenant
-- scope with the constant `PLATFORM_TENANT_ID`. Every other table has the general tenant policy
-- only.

CREATE TABLE audit_log (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    actor_type text NOT NULL,
    actor_id uuid,
    source_ip inet,
    action text NOT NULL,
    object_type text NOT NULL,
    object_id uuid,
    diff jsonb,
    CONSTRAINT audit_log_actor_type_check
        CHECK (actor_type IN ('user', 'api_token', 'agent', 'system')),
    CONSTRAINT audit_log_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id)
);

CREATE FUNCTION audit_log_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'the audit log is append-only';
END
$$;

CREATE TRIGGER audit_log_no_update_delete
    BEFORE UPDATE OR DELETE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_immutable();

CREATE TRIGGER audit_log_no_truncate
    BEFORE TRUNCATE ON audit_log
    FOR EACH STATEMENT EXECUTE FUNCTION audit_log_immutable();

ALTER TABLE audit_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_log FORCE ROW LEVEL SECURITY;

-- A tenant scope reads and writes the rows of its own tenant only.
CREATE POLICY audit_log_tenant ON audit_log
    FOR ALL
    USING (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);

-- The platform scope reads all rows.
CREATE POLICY audit_log_platform_read ON audit_log
    FOR SELECT
    USING (current_setting('restichub.scope', true) = 'platform');

-- The platform scope inserts a record for the entered tenant or for the platform tenant (N2).
-- `enter_tenant` sets `restichub.tenant_id` to the created tenant.
CREATE POLICY audit_log_platform_insert ON audit_log
    FOR INSERT
    WITH CHECK (
        current_setting('restichub.scope', true) = 'platform'
        AND tenant_id IN (
            NULLIF(current_setting('restichub.tenant_id', true), '')::uuid,
            '00000000-0000-7000-8000-000000000000'::uuid
        )
    );
