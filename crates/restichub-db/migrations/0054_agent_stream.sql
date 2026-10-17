-- The live stream of the agent. The control plane instance that holds the stream sets the value at
-- the hello and clears it at the close, so each instance shows the same online state as the
-- dispatcher. A value that an instance crash left stays under the `last_seen_at` window.
ALTER TABLE agents ADD COLUMN stream_id uuid;
