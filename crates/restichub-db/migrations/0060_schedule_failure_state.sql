-- The failure state of a plan or a maintenance schedule (FR-SC-12).
--
-- The scheduler records a failed pass on the row. It never changes the `enabled` switch of the
-- user. `schedule_failure` holds the reason and is null for a healthy row. `failed_since` is the
-- time of the first failed pass of the current failure. `retry_after` keeps a due row out of the
-- claim until the pause of a transient failure ends; the row keeps its fire time, so the missed run
-- policy decides after the recovery (FR-SC-07). A row whose schedule cannot be computed has no fire
-- time and no `retry_after`: it stays failed until the user edits the schedule.
-- `failure_attempts` counts the failed passes and sets the length of the pause.
--
-- The column of migration 0059 held an English sentence. A closed set of reasons replaces it.

ALTER TABLE plans DROP COLUMN schedule_error;
ALTER TABLE plans
    ADD COLUMN schedule_failure text,
    ADD COLUMN failed_since timestamptz,
    ADD COLUMN retry_after timestamptz,
    ADD COLUMN failure_attempts integer NOT NULL DEFAULT 0,
    ADD CONSTRAINT plans_schedule_failure_check CHECK (
        schedule_failure IN ('database_error', 'invalid_cron', 'unknown_time_zone', 'no_fire_time')
    ),
    ADD CONSTRAINT plans_schedule_failure_state_check CHECK (
        (schedule_failure IS NULL) = (failed_since IS NULL)
        AND (schedule_failure IS NOT NULL OR (retry_after IS NULL AND failure_attempts = 0))
    );

ALTER TABLE maintenance_schedules DROP COLUMN schedule_error;
ALTER TABLE maintenance_schedules
    ADD COLUMN schedule_failure text,
    ADD COLUMN failed_since timestamptz,
    ADD COLUMN retry_after timestamptz,
    ADD COLUMN failure_attempts integer NOT NULL DEFAULT 0,
    ADD CONSTRAINT maintenance_schedules_schedule_failure_check CHECK (
        schedule_failure IN ('database_error', 'invalid_cron', 'unknown_time_zone', 'no_fire_time')
    ),
    ADD CONSTRAINT maintenance_schedules_schedule_failure_state_check CHECK (
        (schedule_failure IS NULL) = (failed_since IS NULL)
        AND (schedule_failure IS NOT NULL OR (retry_after IS NULL AND failure_attempts = 0))
    );
