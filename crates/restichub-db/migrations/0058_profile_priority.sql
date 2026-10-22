-- The process and I/O priority of the restic steps of a profile (FR-AG-15, AC-AG-14).
--
-- A profile that exists before this migration keeps the normal priority.
ALTER TABLE profiles ADD COLUMN priority text NOT NULL DEFAULT 'normal';
ALTER TABLE profiles ADD CONSTRAINT profiles_priority_check CHECK (priority IN ('normal', 'low'));
