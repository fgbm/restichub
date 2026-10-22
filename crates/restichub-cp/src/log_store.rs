//! The job log storage (FR-LG-01, FR-LG-02, FR-LG-03, ADR-0015).
//!
//! The [`LogStore`] trait hides the backend. The `database` backend keeps the lines in the
//! `job_logs` table. An `s3` backend (1.0) implements the same trait. Job metadata never goes
//! through the trait: it stays in the database (FR-LG-03).
//!
//! The retention sweep deletes lines older than the retention period of the tenant and leaves the
//! job rows (AC-LG-01). The period comes from the configuration; the default is
//! [`DEFAULT_RETENTION_DAYS`].

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use restichub_db::job_logs::{self, AppendOutcome, LogLine, LogState, NewChunk, NewLine};
use restichub_db::{Database, DbError, JobId, LogLevel, TenantId, tenants};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::supervisor::Supervisor;

/// The days for which the control plane keeps the log text.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// The configuration refuses a longer period: 100 years, far below the range of the date type.
pub const MAX_RETENTION_DAYS: u32 = 36_500;

/// How often the retention sweep runs.
pub const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);

/// Bounds one delete statement, so a sweep never holds a long lock.
const SWEEP_BATCH: i64 = 5000;

/// The most bytes that the control plane accepts in one log chunk. The agent splits longer text.
pub const MAX_CHUNK_BYTES: usize = restichub_proto::MAX_LOG_CHUNK_BYTES;

/// The most characters of one stored line. The database check holds the same value.
const MAX_LINE_CHARS: usize = 8192;

/// One page of log lines and the write state of the log.
#[derive(Debug, Clone)]
pub struct LogPage {
    pub lines: Vec<LogLine>,
    pub state: Option<LogState>,
}

/// Where the text of the job logs lives. Metadata never goes through it.
pub trait LogStore: Send + Sync {
    /// Stores one chunk. A chunk that was stored before changes nothing.
    fn append(
        &self,
        tenant: TenantId,
        job: JobId,
        chunk: &NewChunk,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<AppendOutcome, DbError>> + Send;

    /// Returns up to `limit` lines numbered above `after_seq`, oldest first.
    fn read(
        &self,
        tenant: TenantId,
        job: JobId,
        step_id: Option<&str>,
        after_seq: i64,
        limit: i64,
    ) -> impl Future<Output = Result<LogPage, DbError>> + Send;

    /// Deletes the lines stored before the cutoff and returns their number.
    fn delete_older_than(
        &self,
        tenant: TenantId,
        cutoff: OffsetDateTime,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<i64, DbError>> + Send;
}

/// The `database` backend: the lines live in the `job_logs` table.
#[derive(Debug, Clone)]
pub struct DatabaseLogStore {
    db: Database,
}

impl DatabaseLogStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

impl LogStore for DatabaseLogStore {
    async fn append(
        &self,
        tenant: TenantId,
        job: JobId,
        chunk: &NewChunk,
        now: OffsetDateTime,
    ) -> Result<AppendOutcome, DbError> {
        let mut tx = self.db.begin_tenant(tenant).await?;
        let outcome = job_logs::append(&mut tx, job, chunk, now).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    async fn read(
        &self,
        tenant: TenantId,
        job: JobId,
        step_id: Option<&str>,
        after_seq: i64,
        limit: i64,
    ) -> Result<LogPage, DbError> {
        let mut tx = self.db.begin_tenant(tenant).await?;
        let lines = job_logs::read_step(&mut tx, job, step_id, after_seq, limit).await?;
        let state = job_logs::state(&mut tx, job).await?;
        tx.commit().await?;
        Ok(LogPage { lines, state })
    }

    async fn delete_older_than(
        &self,
        tenant: TenantId,
        cutoff: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<i64, DbError> {
        let mut total = 0;
        loop {
            let mut tx = self.db.begin_tenant(tenant).await?;
            let deleted = job_logs::delete_older_than(&mut tx, cutoff, now, SWEEP_BATCH).await?;
            tx.commit().await?;
            total += deleted;
            if deleted < SWEEP_BATCH {
                return Ok(total);
            }
        }
    }
}

/// Makes the text of a chunk storable and returns one entry for each line.
///
/// The agent scrubs the text before it sends it (FR-AG-06, NFR-SEC-05). The control plane still
/// makes the text safe to store and to show: invalid UTF-8 and control characters never reach a
/// row, and one line is bounded.
pub fn lines_of(data: &[u8]) -> Vec<NewLine> {
    let text = String::from_utf8_lossy(data);
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The text ends with a line break, so the last piece is empty.
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
        .into_iter()
        .map(|line| {
            let text = clean_line(line);
            NewLine {
                level: classify(&text),
                text,
            }
        })
        .collect()
}

/// Removes the control characters except the tab, and the Unicode format characters (category Cf),
/// and cuts the line at the bound.
///
/// The format characters reorder or hide text in the viewer: the bidirectional overrides can make a
/// line read differently from its bytes.
fn clean_line(line: &str) -> String {
    line.chars()
        .filter(|character| {
            *character == '\t' || !(character.is_control() || is_format_character(*character))
        })
        .take(MAX_LINE_CHARS)
        .collect()
}

fn is_format_character(character: char) -> bool {
    matches!(
        character,
        '\u{AD}'
            | '\u{600}'..='\u{605}'
            | '\u{61C}'
            | '\u{6DD}'
            | '\u{70F}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Derives the level of a line from its text. restic prints no level on a plain line.
fn classify(text: &str) -> LogLevel {
    let lower = text.trim_start().to_ascii_lowercase();
    // restic reports an unreadable item as an `error` message and goes on; the run ends with
    // exit code 3 and the status `succeeded_with_warnings`. A fatal error never has this form.
    if lower.starts_with("error during ") {
        LogLevel::Warn
    } else if lower.starts_with("fatal") || lower.starts_with("error") || lower.contains("error:") {
        LogLevel::Error
    } else if lower.starts_with("warning") || lower.contains("warning:") {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

/// Deletes the log lines past the retention of every tenant (FR-LG-02, AC-LG-01).
///
/// The runtime role cannot cross the tenant boundary, so the function lists the tenants and sweeps
/// each tenant in its own transactions. A failure of one tenant is logged and does not stop the
/// others.
pub async fn sweep(
    db: &Database,
    store: &impl LogStore,
    retention: time::Duration,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let tenant_list = {
        let mut tx = db.begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    let cutoff = now - retention;
    for tenant in tenant_list {
        match store.delete_older_than(tenant.id, cutoff, now).await {
            Ok(0) => {}
            Ok(deleted) => {
                tracing::info!(tenant = %tenant.id, deleted, "the log retention removed old lines");
            }
            Err(error) => {
                tracing::warn!(tenant = %tenant.id, %error, "the log retention of a tenant failed");
            }
        }
    }
    Ok(())
}

/// Starts the periodic log retention sweep.
pub fn spawn_retention(
    supervisor: &Supervisor,
    db: Database,
    clock: Arc<dyn Clock>,
    retention_days: u32,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    let store = DatabaseLogStore::new(db.clone());
    let retention = time::Duration::days(i64::from(retention_days));
    supervisor.spawn_loop("log_retention", move |_| {
        let (db, store, clock) = (db.clone(), store.clone(), clock.clone());
        async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(error) = sweep(&db, &store, retention, clock.now()).await {
                    tracing::warn!(%error, "the log retention sweep failed");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_at_the_line_break_and_drop_the_empty_tail() {
        let lines = lines_of(b"one\ntwo\n\nthree\n");
        let texts: Vec<_> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["one", "two", "", "three"]);
    }

    #[test]
    fn control_characters_and_invalid_bytes_never_reach_a_line() {
        let lines = lines_of(b"a\x00b\x1b[31mred\tok\xff\n");
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].text.contains('\0'));
        assert!(!lines[0].text.contains('\u{1b}'));
        assert!(lines[0].text.contains('\t'));
    }

    #[test]
    fn unicode_format_characters_never_reach_a_line() {
        let lines = lines_of("a\u{200B}b\u{202E}c\u{2066}d\u{FEFF}e\tf\n".as_bytes());
        assert_eq!(lines[0].text, "abcde\tf");
    }

    #[test]
    fn a_long_line_is_cut_at_the_bound() {
        let data = "x".repeat(MAX_LINE_CHARS * 2);
        let lines = lines_of(data.as_bytes());
        assert_eq!(lines[0].text.chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn the_level_follows_the_text() {
        assert_eq!(classify("Fatal: wrong password"), LogLevel::Error);
        assert_eq!(classify("error: unable to open"), LogLevel::Error);
        assert_eq!(
            classify("Warning: at least one source file could not be read"),
            LogLevel::Warn
        );
        assert_eq!(classify("scan finished in 1s"), LogLevel::Info);
    }

    #[test]
    fn fr_mn_02_an_item_error_of_a_backup_is_a_warning() {
        assert_eq!(
            classify(
                "error during archival of C:/Users/a/OneDrive/x: Access to the cloud file is denied"
            ),
            LogLevel::Warn
        );
        assert_eq!(classify("error: the repository is locked"), LogLevel::Error);
    }
}
