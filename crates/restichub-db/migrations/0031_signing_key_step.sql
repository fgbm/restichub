-- The signed chain step of a job signing key and the reason for its retirement (ADR-0026, ADR-0030).
--
-- The control plane signs the step that introduces a key once, at the rotation that creates the key,
-- and stores the envelope in the row of the introduced key. The delivery of the chain then reads the
-- stored steps and never unwraps the private key of a retired row (ADR-0030).
--
-- `retired_reason` separates a normal rotation from a compromise. The chain never carries a key that
-- the control plane retired for a compromise, because such a key must not sign a later update.

ALTER TABLE signing_keys
    ADD COLUMN step_update bytea,
    ADD COLUMN retired_reason text;

ALTER TABLE signing_keys
    ADD CONSTRAINT signing_keys_retired_reason_check
    CHECK (retired_reason IS NULL OR retired_reason IN ('rotation', 'compromise'));
