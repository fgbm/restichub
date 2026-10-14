//! The log lines of a job in the `database` log backend (FR-LG-01, FR-LG-02, FR-LG-03, ADR-0015).
//!
//! The log of a job is append-only. The write path refuses a line past [`MAX_JOB_LOG_BYTES`] and
//! marks the log as truncated. The agent numbers the chunks of each stream from 1. The agent sends
//! its log tail again after a restart, so the write path ignores a chunk that is not above the
//! highest stored chunk of its stream. The retention sweep deletes old lines and keeps the job
//! row, the steps and the summaries (AC-LG-01).

use time::OffsetDateTime;

use crate::DbError;
use crate::enums::{LogLevel, LogStream};
use crate::ids::JobId;
use crate::tx::TenantTx;

/// The bytes of log text that one job stores.
///
/// The agent stops at 1 MiB and adds one marker line (FR-AG-08). The margin keeps that marker.
pub const MAX_JOB_LOG_BYTES: i64 = 1024 * 1024 + 4096;

/// The most lines that one read returns.
pub const MAX_READ_LIMIT: i64 = 2000;

/// One line to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLine {
    pub level: LogLevel,
    pub text: String,
}

/// One chunk of the agent: the lines of one step and one stream.
#[derive(Debug, Clone)]
pub struct NewChunk {
    pub step_id: String,
    pub stream: LogStream,
    /// The number of the chunk in its stream, from 1.
    pub chunk: i64,
    pub lines: Vec<NewLine>,
}

/// The result of a chunk write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    /// The number of lines that the write stored. It can be below the lines of the chunk when the
    /// bound cut the chunk.
    Stored(usize),
    /// The chunk was stored before.
    Duplicate,
}

/// One stored line.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct LogLine {
    pub seq: i64,
    pub step_id: String,
    pub stream: LogStream,
    pub level: LogLevel,
    pub ts: OffsetDateTime,
    pub text: String,
}

/// The write state of the log of a job.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct LogState {
    pub last_seq: i64,
    pub bytes: i64,
    pub truncated: bool,
    pub purged_at: Option<OffsetDateTime>,
}

#[derive(sqlx::FromRow)]
struct Marks {
    last_seq: i64,
    bytes: i64,
    truncated: bool,
    stdout_chunk: i64,
    stderr_chunk: i64,
    agent_chunk: i64,
    purged_at: Option<OffsetDateTime>,
}

impl Marks {
    fn chunk(&self, stream: LogStream) -> i64 {
        match stream {
            LogStream::Stdout => self.stdout_chunk,
            LogStream::Stderr => self.stderr_chunk,
            LogStream::Agent => self.agent_chunk,
        }
    }
}

/// Appends the lines of one chunk in one statement.
pub async fn append(
    tx: &mut TenantTx<'_>,
    job: JobId,
    chunk: &NewChunk,
    now: OffsetDateTime,
) -> Result<AppendOutcome, DbError> {
    sqlx::query(
        "INSERT INTO job_log_state (tenant_id, job_id) VALUES ($1, $2) \
         ON CONFLICT (tenant_id, job_id) DO NOTHING",
    )
    .bind(tx.tenant())
    .bind(job)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let marks = sqlx::query_as::<_, Marks>(
        "SELECT last_seq, bytes, truncated, stdout_chunk, stderr_chunk, agent_chunk, purged_at \
         FROM job_log_state WHERE job_id = $1 FOR UPDATE",
    )
    .bind(job)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    if chunk.chunk <= marks.chunk(chunk.stream) {
        return Ok(AppendOutcome::Duplicate);
    }

    let mut bytes = marks.bytes;
    let mut truncated = marks.truncated;
    let mut levels = Vec::with_capacity(chunk.lines.len());
    let mut texts = Vec::with_capacity(chunk.lines.len());
    // A purged log takes no new line: the sweep removed the lines of an old job.
    if marks.purged_at.is_none() {
        for line in &chunk.lines {
            let size = i64::try_from(line.text.len()).unwrap_or(i64::MAX);
            if bytes.saturating_add(size) > MAX_JOB_LOG_BYTES {
                truncated = true;
                break;
            }
            bytes += size;
            levels.push(line.level.as_str());
            texts.push(line.text.as_str());
        }
    }
    let stored = texts.len();
    if stored > 0 {
        sqlx::query(
            "INSERT INTO job_logs (tenant_id, job_id, seq, step_id, stream, level, ts, text) \
             SELECT $1, $2, $3 + u.n, $4, $5, u.level, $6, u.text \
             FROM unnest($7::text[], $8::text[]) WITH ORDINALITY AS u(level, text, n)",
        )
        .bind(tx.tenant())
        .bind(job)
        .bind(marks.last_seq)
        .bind(&chunk.step_id)
        .bind(chunk.stream)
        .bind(now)
        .bind(&levels)
        .bind(&texts)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    }
    let update = match chunk.stream {
        LogStream::Stdout => {
            "UPDATE job_log_state SET last_seq = last_seq + $3, bytes = $4, truncated = $5, \
             stdout_chunk = $6 WHERE job_id = $1 AND tenant_id = $2"
        }
        LogStream::Stderr => {
            "UPDATE job_log_state SET last_seq = last_seq + $3, bytes = $4, truncated = $5, \
             stderr_chunk = $6 WHERE job_id = $1 AND tenant_id = $2"
        }
        LogStream::Agent => {
            "UPDATE job_log_state SET last_seq = last_seq + $3, bytes = $4, truncated = $5, \
             agent_chunk = $6 WHERE job_id = $1 AND tenant_id = $2"
        }
    };
    sqlx::query(update)
        .bind(job)
        .bind(tx.tenant())
        .bind(i64::try_from(stored).unwrap_or(i64::MAX))
        .bind(bytes)
        .bind(truncated)
        .bind(chunk.chunk)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(AppendOutcome::Stored(stored))
}

/// Reads the lines of a job after a line number, oldest first.
pub async fn read(
    tx: &mut TenantTx<'_>,
    job: JobId,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<LogLine>, DbError> {
    sqlx::query_as::<_, LogLine>(
        "SELECT seq, step_id, stream, level, ts, text FROM job_logs \
         WHERE job_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
    )
    .bind(job)
    .bind(after_seq)
    .bind(limit.clamp(1, MAX_READ_LIMIT))
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Reads the write state of the log of a job. A job without a stored line has none.
pub async fn state(tx: &mut TenantTx<'_>, job: JobId) -> Result<Option<LogState>, DbError> {
    sqlx::query_as::<_, LogState>(
        "SELECT last_seq, bytes, truncated, purged_at FROM job_log_state WHERE job_id = $1",
    )
    .bind(job)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Deletes up to `batch` lines that the control plane stored before `cutoff`.
///
/// The jobs that lost lines get `purged_at`. The job rows, the steps and the summaries stay
/// (AC-LG-01). The caller repeats the call while it returns `batch` lines.
pub async fn delete_older_than(
    tx: &mut TenantTx<'_>,
    cutoff: OffsetDateTime,
    now: OffsetDateTime,
    batch: i64,
) -> Result<i64, DbError> {
    let deleted: i64 = sqlx::query_scalar(
        "WITH gone AS ( \
             DELETE FROM job_logs WHERE (tenant_id, job_id, seq) IN ( \
                 SELECT tenant_id, job_id, seq FROM job_logs \
                 WHERE tenant_id = $1 AND ts < $2 LIMIT $4) \
             RETURNING job_id \
         ), hit AS (SELECT job_id, count(*) AS n FROM gone GROUP BY job_id), \
         marked AS ( \
             UPDATE job_log_state s SET purged_at = $3 FROM hit \
             WHERE s.job_id = hit.job_id AND s.tenant_id = $1 RETURNING 1 \
         ) \
         SELECT COALESCE(sum(n), 0)::bigint FROM hit",
    )
    .bind(tx.tenant())
    .bind(cutoff)
    .bind(now)
    .bind(batch)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(deleted)
}
