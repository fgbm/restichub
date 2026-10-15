-- Certificate rotation and the protocol version of an agent (ADR-0026, FR-ID-03, FR-PR-09).
--
-- A rotation over the stream issues a new certificate while the old one stays valid. The registry
-- row keeps the issued certificate as pending until the agent confirms it over the stream or
-- connects with it; the control plane then promotes it and refuses the old serial. The serial of a
-- pending certificate is unique like the serial of an enrolled one.
--
-- The row also records the last rotation and the protocol version of the last hello. The Hosts
-- screen reads them.

ALTER TABLE agents ADD COLUMN pending_certificate_serial bytea;
ALTER TABLE agents ADD COLUMN pending_certificate_expires_at timestamptz;
ALTER TABLE agents ADD COLUMN last_certificate_rotation_at timestamptz;
ALTER TABLE agents ADD COLUMN protocol_version_major integer;
ALTER TABLE agents ADD COLUMN protocol_version_minor integer;

ALTER TABLE agents ADD CONSTRAINT agents_pending_certificate_check CHECK (
    (pending_certificate_serial IS NULL) = (pending_certificate_expires_at IS NULL)
);

CREATE UNIQUE INDEX agents_pending_certificate_serial_unique
    ON agents (pending_certificate_serial)
    WHERE pending_certificate_serial IS NOT NULL;
