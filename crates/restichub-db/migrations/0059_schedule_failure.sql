-- The cause of the last failed scheduler pass of a plan or a maintenance schedule (FR-SC-02).
--
-- The scheduler records the cause when a pass fails and moves the fire time forward or disables the
-- row. A pass that succeeds clears the cause, and so does an edit of the row. The metrics endpoint
-- shows each row that holds a cause.

ALTER TABLE plans ADD COLUMN schedule_error text;

ALTER TABLE maintenance_schedules ADD COLUMN schedule_error text;
