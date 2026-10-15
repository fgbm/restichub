-- A script variable of the kind `secret_ref` holds the identifier of a secret. The control plane
-- now saves it in the canonical form (lower case, with hyphens), and the usage of a secret is found
-- by the structure of the script JSON. This migration brings the saved references to that form. A
-- value that is not an identifier, for example the empty reference of a job template, stays as is.

CREATE FUNCTION canonical_script_secret_refs(scripts jsonb) RETURNS jsonb
    LANGUAGE sql IMMUTABLE AS $$
    SELECT COALESCE(jsonb_agg(
        CASE WHEN jsonb_typeof(script -> 'env') = 'array' THEN
            jsonb_set(script, '{env}', (
                SELECT COALESCE(jsonb_agg(
                    CASE
                        WHEN variable ->> 'kind' = 'secret_ref'
                            AND regexp_replace(
                                    regexp_replace(lower(variable ->> 'value'), '^urn:uuid:', ''),
                                    '[{}-]', '', 'g') ~ '^[0-9a-f]{32}$'
                        THEN jsonb_set(variable, '{value}', to_jsonb((regexp_replace(
                                regexp_replace(lower(variable ->> 'value'), '^urn:uuid:', ''),
                                '[{}-]', '', 'g'))::uuid::text))
                        ELSE variable
                    END ORDER BY position), '[]'::jsonb)
                FROM jsonb_array_elements(script -> 'env') WITH ORDINALITY AS e(variable, position)
            ))
        ELSE script END ORDER BY script_position), '[]'::jsonb)
    FROM jsonb_array_elements(scripts) WITH ORDINALITY AS s(script, script_position)
$$;

DO $$
DECLARE
    owner uuid;
BEGIN
    PERFORM set_config('restichub.scope', 'platform', true);
    FOR owner IN SELECT id FROM tenants LOOP
        PERFORM set_config('restichub.tenant_id', owner::text, true);
        UPDATE profiles SET scripts = canonical_script_secret_refs(scripts)
            WHERE jsonb_array_length(scripts) > 0
              AND scripts IS DISTINCT FROM canonical_script_secret_refs(scripts);
        UPDATE job_templates SET scripts = canonical_script_secret_refs(scripts)
            WHERE jsonb_array_length(scripts) > 0
              AND scripts IS DISTINCT FROM canonical_script_secret_refs(scripts);
    END LOOP;
    PERFORM set_config('restichub.tenant_id', '', true);
    PERFORM set_config('restichub.scope', '', true);
END $$;

DROP FUNCTION canonical_script_secret_refs(jsonb);
