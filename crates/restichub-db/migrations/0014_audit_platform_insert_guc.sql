-- The platform audit insert for an existing tenant (issue #20, FR-TN-02).
--
-- A platform operation on an existing tenant (suspend, resume or delete) writes the audit record of
-- the affected tenant. Migration 0006 read the entered tenant from `restichub.tenant_id`. That GUC
-- also selects the tenant scope of every other table, so a platform transaction that writes the
-- audit record of an existing tenant would then read and write the rows of that tenant.
--
-- This migration moves that write to a separate GUC `restichub.audit_tenant_id`. Only the policy
-- below reads it. The function `audit::insert_platform_for` sets it for one insert and clears it
-- again. The transaction keeps no tenant scope after the write.

DROP POLICY audit_log_platform_insert ON audit_log;

CREATE POLICY audit_log_platform_insert ON audit_log
    FOR INSERT
    WITH CHECK (
        current_setting('restichub.scope', true) = 'platform'
        AND tenant_id IN (
            NULLIF(current_setting('restichub.tenant_id', true), '')::uuid,
            NULLIF(current_setting('restichub.audit_tenant_id', true), '')::uuid,
            '00000000-0000-7000-8000-000000000000'::uuid
        )
    );
