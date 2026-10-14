use super::*;
use crate::JobId;
use crate::enums::{LogLevel, LogStream, StepStatus};
use crate::job_logs::{self, AppendOutcome, NewChunk, NewLine};
use crate::job_steps::{self, BackupSummary, JobStep, StepOutput};
use crate::test_util::test_database;
use time::{Duration, OffsetDateTime};

/// A tenant with one job and no stored log, step or progress row.
async fn clean_job(db: &Database, tenant: TenantId) -> Result<JobId, Box<dyn std::error::Error>> {
    seed_tenant(db, tenant).await?;
    let mut tx = db.begin_tenant(tenant).await?;
    for table in ["job_logs", "job_log_state", "job_steps", "job_progress"] {
        sqlx::query(AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(tx.as_conn())
            .await?;
    }
    let (job,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM jobs")
        .fetch_one(tx.as_conn())
        .await?;
    tx.commit().await?;
    Ok(JobId::from_uuid(job))
}

fn chunk(stream: LogStream, number: i64, texts: &[&str]) -> NewChunk {
    NewChunk {
        step_id: "backup".to_owned(),
        stream,
        chunk: number,
        lines: texts
            .iter()
            .map(|text| NewLine {
                level: LogLevel::Info,
                text: (*text).to_owned(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn ac_lg_01_db_a_resent_chunk_stores_no_line_twice() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _other) = two_tenants(db.database()).await?;
    let job = clean_job(db.database(), tenant).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    let first = chunk(LogStream::Stdout, 1, &["a", "b"]);
    let second = chunk(LogStream::Stdout, 2, &["c"]);
    assert_eq!(
        job_logs::append(&mut tx, job, &first, now).await?,
        AppendOutcome::Stored(2)
    );
    assert_eq!(
        job_logs::append(&mut tx, job, &second, now).await?,
        AppendOutcome::Stored(1)
    );
    // The tail that the agent sends again after a restart.
    assert_eq!(
        job_logs::append(&mut tx, job, &first, now).await?,
        AppendOutcome::Duplicate
    );
    assert_eq!(
        job_logs::append(&mut tx, job, &second, now).await?,
        AppendOutcome::Duplicate
    );
    // The streams count their chunks apart.
    assert_eq!(
        job_logs::append(&mut tx, job, &chunk(LogStream::Stderr, 1, &["e"]), now).await?,
        AppendOutcome::Stored(1)
    );
    let lines = job_logs::read(&mut tx, job, 0, 100).await?;
    let texts: Vec<_> = lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(texts, ["a", "b", "c", "e"]);
    assert_eq!(
        lines.iter().map(|line| line.seq).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    let page = job_logs::read(&mut tx, job, 2, 1).await?;
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].text, "c");
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn fr_ag_08_db_the_log_of_a_job_is_bounded() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _other) = two_tenants(db.database()).await?;
    let job = clean_job(db.database(), tenant).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    let big = "x".repeat(8000);
    let texts: Vec<&str> = std::iter::repeat_n(big.as_str(), 200).collect();
    // 200 lines of 8000 bytes pass the bound of 1 MiB plus the margin.
    let outcome = job_logs::append(&mut tx, job, &chunk(LogStream::Stdout, 1, &texts), now).await?;
    let AppendOutcome::Stored(stored) = outcome else {
        panic!("the chunk must be stored in part");
    };
    assert!(
        stored < 200,
        "the bound must cut the chunk, stored {stored}"
    );
    let state = job_logs::state(&mut tx, job)
        .await?
        .expect("the state exists");
    assert!(state.truncated);
    assert!(state.bytes <= job_logs::MAX_JOB_LOG_BYTES);
    // A later chunk adds no line.
    assert_eq!(
        job_logs::append(
            &mut tx,
            job,
            &chunk(LogStream::Stdout, 2, &[big.as_str()]),
            now
        )
        .await?,
        AppendOutcome::Stored(0)
    );
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_lg_01_db_the_retention_deletes_old_lines_and_keeps_the_job() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _other) = two_tenants(db.database()).await?;
    let job = clean_job(db.database(), tenant).await?;
    let now = OffsetDateTime::now_utc();
    let mut tx = db.database().begin_tenant(tenant).await?;
    job_logs::append(
        &mut tx,
        job,
        &chunk(LogStream::Stdout, 1, &["old"]),
        now - Duration::days(8),
    )
    .await?;
    job_logs::append(&mut tx, job, &chunk(LogStream::Stdout, 2, &["new"]), now).await?;
    job_steps::upsert(
        &mut tx,
        job,
        &JobStep {
            step_id: "backup".to_owned(),
            status: StepStatus::Success,
            exit_code: 0,
            started_at: None,
            finished_at: None,
            error_code: None,
            error_params: Default::default(),
            possible_stale_lock: false,
            output: StepOutput::None,
        },
    )
    .await?;
    let deleted = job_logs::delete_older_than(&mut tx, now - Duration::days(7), now, 100).await?;
    assert_eq!(deleted, 1);
    let lines = job_logs::read(&mut tx, job, 0, 100).await?;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "new");
    let state = job_logs::state(&mut tx, job)
        .await?
        .expect("the state exists");
    assert!(state.purged_at.is_some());
    // The job row and its steps stay.
    assert!(crate::jobs::get(&mut tx, job).await?.is_some());
    assert_eq!(job_steps::list(&mut tx, job).await?.len(), 1);
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_mn_04_db_a_backup_summary_round_trips_and_a_second_write_keeps_it() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _other) = two_tenants(db.database()).await?;
    let job = clean_job(db.database(), tenant).await?;
    let summary = BackupSummary {
        files_new: 12,
        files_changed: 3,
        files_unmodified: 100,
        dirs_new: 2,
        dirs_changed: 1,
        dirs_unmodified: 9,
        data_blobs: 14,
        tree_blobs: 3,
        data_added: 123_456,
        data_added_packed: 100_000,
        total_files_processed: 115,
        total_bytes_processed: 9_999_999,
        total_duration_seconds: 4.5,
        snapshot_id: "abcdef12".to_owned(),
        dry_run: false,
    };
    let step = |output| JobStep {
        step_id: "backup".to_owned(),
        status: StepStatus::Success,
        exit_code: 0,
        started_at: None,
        finished_at: None,
        error_code: None,
        error_params: Default::default(),
        possible_stale_lock: false,
        output,
    };
    let mut tx = db.database().begin_tenant(tenant).await?;
    assert!(job_steps::upsert(&mut tx, job, &step(StepOutput::Backup(summary.clone()))).await?);
    // The journal copy of the result drops the output. It must not erase the summary.
    assert!(job_steps::upsert(&mut tx, job, &step(StepOutput::None)).await?);
    let steps = job_steps::list(&mut tx, job).await?;
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].output, StepOutput::Backup(summary));
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_tn_01_db_a_log_row_refuses_a_job_of_another_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let job_a = clean_job(db.database(), tenant_a).await?;
    clean_job(db.database(), tenant_b).await?;
    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let result = sqlx::query(
        "INSERT INTO job_logs (tenant_id, job_id, seq, stream, level, ts, text) \
         VALUES ($1, $2, 1, 'stdout', 'info', now(), 'x')",
    )
    .bind(tenant_b)
    .bind(job_a)
    .execute(tx.as_conn())
    .await;
    assert_code(result, "23503");
    drop(tx);
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_mn_03_db_the_metrics_snapshot_reads_the_tenant_rows() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    clean_job(db.database(), tenant_a).await?;
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let snapshot = crate::metrics::snapshot(&mut tx).await?;
    assert_eq!(snapshot.agents.len(), 1);
    assert_eq!(snapshot.repo_lock_held.len(), 1);
    assert_eq!(snapshot.jobs_total.len(), 4);
    tx.commit().await?;
    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let empty = crate::metrics::snapshot(&mut tx).await?;
    assert!(empty.agents.is_empty() && empty.repo_lock_held.is_empty());
    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}
