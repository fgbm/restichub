-- The audit read indexes (issue #19, FR-AU-01).
--
-- The audit list reads the records in the keyset order of migration 0006:
-- `occurred_at DESC, id DESC`. The tenant read also filters by `tenant_id`, so the first index
-- serves it. The platform read reads every tenant, so the second index serves it.
-- The table stays append-only. An index is not a table, so the guard tests do not change.

CREATE INDEX audit_log_tenant_time_id_idx
    ON audit_log (tenant_id, occurred_at DESC, id DESC);

CREATE INDEX audit_log_time_id_idx
    ON audit_log (occurred_at DESC, id DESC);
