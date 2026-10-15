-- Re-registration, duplicate detection and the waiting state of a host (ADR-0026, FR-ID-01,
-- FR-ID-04, FR-TN-03).
--
-- `first_connected_at` is the time of the first stream open. An active host with no value waits for
-- its first connection. A host that opened a stream before this migration keeps the evidence in
-- `last_seen_at`: the enrollment wrote that column with the creation time, so a value later than
-- the creation time means a connection.
--
-- `machine_id_hash` is a tenant-keyed hash of the machine identifier that the agent reports at the
-- enrollment. It is a hint for the duplicate warning only; no decision of the control plane about
-- identity depends on it.
--
-- `replaces_agent_id` binds an enrollment token to an existing host. The enrollment with such a
-- token keeps the agent identifier and replaces the certificate.

ALTER TABLE agents ADD COLUMN first_connected_at timestamptz;
ALTER TABLE agents ADD COLUMN machine_id_hash bytea;

UPDATE agents
   SET first_connected_at = last_seen_at
 WHERE last_seen_at IS NOT NULL
   AND last_seen_at > created_at + interval '5 seconds';

CREATE INDEX agents_machine_id_hash_idx
    ON agents (tenant_id, machine_id_hash)
    WHERE machine_id_hash IS NOT NULL;

ALTER TABLE enrollment_tokens ADD COLUMN replaces_agent_id uuid;
ALTER TABLE enrollment_tokens ADD CONSTRAINT enrollment_tokens_replaces_agent_fk
    FOREIGN KEY (tenant_id, replaces_agent_id) REFERENCES agents (tenant_id, id);

-- `used_by_agent_id` names the host that a spent token enrolled or re-registered.
ALTER TABLE enrollment_tokens ADD COLUMN used_by_agent_id uuid;
ALTER TABLE enrollment_tokens ADD CONSTRAINT enrollment_tokens_used_by_agent_fk
    FOREIGN KEY (tenant_id, used_by_agent_id) REFERENCES agents (tenant_id, id);

-- The function returns one more column, so it is dropped and created again with the same owner and
-- grants as in migration 0025.
GRANT CREATE ON SCHEMA public TO restichub_auth;

DROP FUNCTION enroll_consume_token(bytea);

CREATE FUNCTION enroll_consume_token(token bytea)
RETURNS TABLE (outcome text, tenant_id uuid, token_id uuid, labels text[], replaces_agent_id uuid)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    found_id uuid;
    found_tenant uuid;
    found_labels text[];
    found_replaces uuid;
    found_used timestamptz;
    found_revoked timestamptz;
    found_expires timestamptz;
BEGIN
    SELECT entry.id, entry.tenant_id, entry.labels, entry.replaces_agent_id, entry.used_at,
           entry.revoked_at, entry.expires_at
      INTO found_id, found_tenant, found_labels, found_replaces, found_used, found_revoked,
           found_expires
      FROM public.enrollment_tokens AS entry
     WHERE entry.token_hash = token
       FOR UPDATE;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    IF found_used IS NOT NULL THEN
        RETURN QUERY SELECT 'used'::text, found_tenant, found_id, found_labels, found_replaces;
        RETURN;
    END IF;
    IF found_revoked IS NOT NULL THEN
        RETURN QUERY SELECT 'revoked'::text, found_tenant, found_id, found_labels, found_replaces;
        RETURN;
    END IF;
    IF found_expires <= now() THEN
        RETURN QUERY SELECT 'expired'::text, found_tenant, found_id, found_labels, found_replaces;
        RETURN;
    END IF;
    UPDATE public.enrollment_tokens AS entry SET used_at = now() WHERE entry.id = found_id;
    RETURN QUERY SELECT 'consumed'::text, found_tenant, found_id, found_labels, found_replaces;
END
$$;

ALTER FUNCTION enroll_consume_token(bytea) OWNER TO restichub_auth;

REVOKE ALL ON FUNCTION enroll_consume_token(bytea) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION enroll_consume_token(bytea) TO restichub_app;

REVOKE CREATE ON SCHEMA public FROM restichub_auth;
