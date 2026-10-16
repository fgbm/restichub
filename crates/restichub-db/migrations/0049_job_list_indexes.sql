-- The indexes of the filtered job list (FR-SC-09, FR-UI-01, ADR-0009).
--
-- The list always leaves out the browse jobs of the Restore screen, so each index holds only the
-- rows that the list can show. Each index ends with the default order of the list, so a filter on
-- one column reads the newest rows first and stops after the page. The host filter uses
-- `jobs_tenant_agent_idx` (0028), and the list without a filter uses `jobs_tenant_created_idx`
-- (0028).

CREATE INDEX jobs_list_plan_idx ON jobs (tenant_id, plan_id, created_at DESC, id DESC)
    WHERE kind NOT IN ('snapshots', 'ls');

CREATE INDEX jobs_list_repository_idx ON jobs (tenant_id, repository_id, created_at DESC, id DESC)
    WHERE kind NOT IN ('snapshots', 'ls');

CREATE INDEX jobs_list_status_idx ON jobs (tenant_id, status, created_at DESC, id DESC)
    WHERE kind NOT IN ('snapshots', 'ls');

CREATE INDEX jobs_list_started_idx
    ON jobs (tenant_id, started_at DESC NULLS LAST, created_at DESC, id DESC)
    WHERE kind NOT IN ('snapshots', 'ls');
