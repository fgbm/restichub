-- The initial migration of the control plane database.
-- The real schema arrives in the next task. This migration only makes the migration runner work.
-- Each table has tenant_id (AGENTS.md).

CREATE TABLE IF NOT EXISTS schema_info (
    tenant_id uuid NOT NULL,
    key text NOT NULL,
    value text NOT NULL,
    PRIMARY KEY (tenant_id, key)
);
