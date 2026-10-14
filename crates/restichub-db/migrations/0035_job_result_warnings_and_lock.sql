ALTER TABLE jobs DROP CONSTRAINT jobs_result_code_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_result_code_check CHECK (
    result_code IS NULL
    OR result_code IN (
        'succeeded', 'failed', 'config_error', 'not_initialized', 'cancelled', 'access_error',
        'succeeded_with_warnings', 'repository_locked'
    )
);
