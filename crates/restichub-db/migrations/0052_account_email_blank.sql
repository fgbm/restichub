-- An account without an email holds NULL, never an empty string (ADR-0028).
--
-- The unique email index ignores NULL, so any number of accounts can have no email. Two empty
-- strings collide in the index, so a blank value becomes NULL and the check refuses it after.

-- The owner login is subject to FORCE ROW LEVEL SECURITY and the update must see every row.
ALTER TABLE users NO FORCE ROW LEVEL SECURITY;

UPDATE users SET email = NULL WHERE btrim(email) = '';

ALTER TABLE users
    ADD CONSTRAINT users_email_not_blank CHECK (email IS NULL OR btrim(email) <> '');

ALTER TABLE users FORCE ROW LEVEL SECURITY;
