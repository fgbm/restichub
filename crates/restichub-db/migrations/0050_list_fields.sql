-- The facts that the entity lists show in each item (FR-UI-01, ADR-0036).
--
-- `alert_deliveries.attempted_at` is the end of the last attempt of a delivery, sent or failed. The
-- channel list shows the last attempt and its result. A row of an earlier version keeps a null
-- value and does not count.
--
-- `tenant_agent_counts()` gives the platform list of the organizations the number of the hosts
-- that are not revoked. It follows the rules of migration 0013: the platform scope cannot read the
-- `agents` table, so one narrow `SECURITY DEFINER` function returns the count only and no agent row.
--
-- The indexes serve one item fact each. The pull request of this migration holds their plans.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'restichub_auth') THEN
        RAISE EXCEPTION
            'the role restichub_auth is missing; run crates/restichub-db/sql/bootstrap-roles.sql as a database administrator';
    END IF;
END
$$;

ALTER TABLE alert_deliveries ADD COLUMN attempted_at timestamptz;

-- The last attempt of each channel.
CREATE INDEX alert_deliveries_channel_attempt_idx
    ON alert_deliveries (tenant_id, channel_id, attempted_at DESC, id DESC)
    WHERE attempted_at IS NOT NULL;

-- The last check of each repository. A check is rare beside the backups of the same repository,
-- so the list index of 0049 reads many rows before it finds one.
CREATE INDEX jobs_repository_check_idx ON jobs (tenant_id, repository_id, finished_at DESC)
    WHERE kind = 'check' AND status IN ('succeeded', 'failed', 'dead_letter');

-- The last sign-in of each member. The audit log of a tenant holds many more records of other
-- actions, for example one for each delivered secret.
CREATE INDEX audit_log_sign_in_idx ON audit_log (tenant_id, actor_id, occurred_at DESC)
    WHERE action IN ('sign_in', 'session_switch');

GRANT USAGE, CREATE ON SCHEMA public TO restichub_auth;
GRANT SELECT ON agents TO restichub_auth;

CREATE FUNCTION tenant_agent_counts()
RETURNS TABLE (tenant_id uuid, agent_count bigint)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT agent.tenant_id, count(*)::bigint
    FROM public.agents AS agent
    WHERE current_setting('restichub.scope', true) IS NOT DISTINCT FROM 'platform'
      AND agent.status <> 'revoked'
    GROUP BY agent.tenant_id
$$;

ALTER FUNCTION tenant_agent_counts() OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION tenant_agent_counts() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION tenant_agent_counts() TO restichub_app;

-- The definer role creates no object after the migration (N5). It keeps USAGE.
REVOKE CREATE ON SCHEMA public FROM restichub_auth;
