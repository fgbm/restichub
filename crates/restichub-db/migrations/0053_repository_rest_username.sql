-- The user name of the rest-server account (FR-RP-01). It is not a secret, so the row holds the
-- value; the password stays in the `secrets` table (ADR-0017). Only a rest backend holds it.
ALTER TABLE repositories ADD COLUMN rest_username text;

ALTER TABLE repositories
    ADD CONSTRAINT repositories_rest_username_check CHECK (
        rest_username IS NULL OR (backend = 'rest' AND btrim(rest_username) <> '')
    );
