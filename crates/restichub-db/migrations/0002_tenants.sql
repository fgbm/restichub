-- Tenants and the reserved platform tenant (ADR-0006, ADR-0023, issue #16).
--
-- This migration replaces the placeholder table `schema_info` with the real schema.
-- The reserved platform tenant has the fixed UUID `00000000-0000-7000-8000-000000000000`. The
-- Rust code exports the same value as `PLATFORM_TENANT_ID`.

DROP TABLE IF EXISTS schema_info;

CREATE TABLE tenants (
    id uuid NOT NULL PRIMARY KEY,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    status text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT tenants_status_check CHECK (status IN ('active', 'suspended', 'deleted')),
    CONSTRAINT tenants_tenant_id_fk FOREIGN KEY (tenant_id) REFERENCES tenants (id),
    CONSTRAINT tenants_tenant_id_self_check CHECK (tenant_id = id)
);

-- The name is unique and the comparison ignores the letter case (D6).
CREATE UNIQUE INDEX tenants_name_lower_unique ON tenants (lower(name));

-- The reserved platform tenant. The Platform Admin users and the platform audit records belong to
-- this tenant. The row is inserted before Row-Level Security is enabled.
INSERT INTO tenants (id, tenant_id, name)
VALUES (
    '00000000-0000-7000-8000-000000000000',
    '00000000-0000-7000-8000-000000000000',
    'platform'
);

ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenants FORCE ROW LEVEL SECURITY;

-- The platform scope can use every row of `tenants`.
CREATE POLICY tenants_platform ON tenants
    FOR ALL
    USING (current_setting('restichub.scope', true) = 'platform')
    WITH CHECK (current_setting('restichub.scope', true) = 'platform');

-- A tenant scope can read its own row only.
CREATE POLICY tenants_self_read ON tenants
    FOR SELECT
    USING (id = NULLIF(current_setting('restichub.tenant_id', true), '')::uuid);
