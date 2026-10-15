-- The typed values behind the result message of a job, for example the option and the versions of
-- an incompatible job (AC-AG-09). The user interface builds its text from them.
ALTER TABLE jobs ADD COLUMN result_params jsonb;
ALTER TABLE jobs ADD CONSTRAINT jobs_result_params_check CHECK (
    result_params IS NULL OR jsonb_typeof(result_params) = 'object'
);
