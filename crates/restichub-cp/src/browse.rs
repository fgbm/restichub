//! The listings of the Restore screen: snapshots and one directory of a snapshot (FR-RS-01,
//! FR-RS-02, ADR-0035).
//!
//! The agent sends the list of a `snapshots` step or a page of an `ls` step in the step result. The
//! control plane stores a bounded and cleaned copy in `job_listings`, because the journal copy of
//! the agent drops these outputs (AC-PR-07). The same types answer the API, so the stored value
//! and the response agree. A listing comes from an agent, so each value is cut to a bound and has
//! no control character (FR-PR-08).

use restichub_db::restore as restore_repo;
use restichub_db::{JobKind, JobStatus, TenantTx, jobs::Job};
use restichub_proto::agent::v1::{LsPage, Node, NodeType, Snapshot, SnapshotList, step_result};
use restichub_proto::validate::MAX_LS_LIMIT;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::job_data::{clean, time_of};
use crate::jobs::format_time;

/// The most snapshots in one stored list. The agent sends at most as many (FR-RS-01).
const MAX_SNAPSHOTS: usize = 2_000;

/// The most entries of the lists inside one snapshot, such as its tags and its paths.
const MAX_LIST_ITEMS: usize = 64;

const MAX_ID_CHARS: usize = 64;
const MAX_TEXT_CHARS: usize = 256;
const MAX_PATH_CHARS: usize = 4_096;
const MAX_TIME_CHARS: usize = 64;

/// Progress of a browse request at the moment of the read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowseState {
    /// No listing exists yet and no job is named.
    Empty,
    /// The job is queued or runs. Ask again.
    Pending,
    /// The listing is ready in `snapshots` or `nodes`.
    Done,
    /// The job ended without a listing. `error_code` says why.
    Failed,
}

/// One snapshot of a repository, as `restic snapshots --json` reports it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotItem {
    /// The 64-character identifier. `restore` and `ls` take it.
    pub id: String,
    pub short_id: String,
    /// In RFC 3339. Null when restic gave no valid time.
    pub time: Option<String>,
    pub hostname: String,
    pub username: String,
    pub tags: Vec<String>,
    /// The paths that the backup read, as the backup recorded them.
    pub paths: Vec<String>,
    pub program_version: String,
    /// Null when restic gave no backup summary.
    pub total_files: Option<i64>,
    /// Null when restic gave no backup summary.
    pub total_bytes: Option<i64>,
}

/// Tells how the screen shows an entry of a snapshot tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
    /// A device, a pipe, a socket or an entry of an unknown type.
    Other,
}

/// One entry of a snapshot tree, as `restic ls --json` reports it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeItem {
    pub name: String,
    pub kind: NodeKind,
    /// The path of the entry from the root of the snapshot.
    pub path: String,
    /// In bytes.
    pub size: i64,
    pub permissions: String,
    /// In RFC 3339. Null when restic gave no valid time.
    pub mtime: Option<String>,
    /// Empty unless the entry is a symbolic link.
    pub link_target: String,
}

/// Value that the control plane stores for a `snapshots` job.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredSnapshots {
    pub truncated: bool,
    pub snapshots: Vec<SnapshotItem>,
}

/// Value that the control plane stores for an `ls` job.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPage {
    pub last: bool,
    pub nodes: Vec<NodeItem>,
}

fn timestamp(value: Option<&prost_types::Timestamp>) -> Option<String> {
    let time = time_of(value?)?;
    let text = format_time(time);
    (text.chars().count() <= MAX_TIME_CHARS).then_some(text)
}

fn strings(values: &[String], max_chars: usize) -> Vec<String> {
    values
        .iter()
        .take(MAX_LIST_ITEMS)
        .map(|value| clean(value, max_chars))
        .collect()
}

fn snapshot_item(snapshot: &Snapshot) -> SnapshotItem {
    SnapshotItem {
        id: clean(&snapshot.id, MAX_ID_CHARS),
        short_id: clean(&snapshot.short_id, MAX_ID_CHARS),
        time: timestamp(snapshot.time.as_ref()),
        hostname: clean(&snapshot.hostname, MAX_TEXT_CHARS),
        username: clean(&snapshot.username, MAX_TEXT_CHARS),
        tags: strings(&snapshot.tags, MAX_TEXT_CHARS),
        paths: strings(&snapshot.paths, MAX_PATH_CHARS),
        program_version: clean(&snapshot.program_version, MAX_TEXT_CHARS),
        total_files: snapshot
            .summary
            .as_ref()
            .map(|summary| i64::try_from(summary.total_files_processed).unwrap_or(i64::MAX)),
        total_bytes: snapshot
            .summary
            .as_ref()
            .map(|summary| i64::try_from(summary.total_bytes_processed).unwrap_or(i64::MAX)),
    }
}

fn node_item(node: &Node) -> NodeItem {
    let kind = match NodeType::try_from(node.r#type) {
        Ok(NodeType::File) => NodeKind::File,
        Ok(NodeType::Dir) => NodeKind::Dir,
        Ok(NodeType::Symlink) => NodeKind::Symlink,
        _ => NodeKind::Other,
    };
    NodeItem {
        name: clean(&node.name, MAX_PATH_CHARS),
        kind,
        path: clean(&node.path, MAX_PATH_CHARS),
        size: node.size.max(0),
        permissions: clean(&node.permissions, 16),
        mtime: timestamp(node.mtime.as_ref()),
        link_target: clean(&node.link_target, MAX_PATH_CHARS),
    }
}

/// Bounds a snapshot list from an agent. The newest snapshots stay (the agent keeps the order).
pub fn stored_snapshots(list: &SnapshotList) -> StoredSnapshots {
    let skip = list.snapshots.len().saturating_sub(MAX_SNAPSHOTS);
    StoredSnapshots {
        truncated: list.truncated || skip > 0,
        snapshots: list
            .snapshots
            .iter()
            .skip(skip)
            .map(snapshot_item)
            .collect(),
    }
}

/// Bounds a page of nodes from an agent.
pub fn stored_page(page: &LsPage) -> StoredPage {
    let max = usize::try_from(MAX_LS_LIMIT).unwrap_or(usize::MAX);
    StoredPage {
        last: page.last && page.nodes.len() <= max,
        nodes: page.nodes.iter().take(max).map(node_item).collect(),
    }
}

/// Stores the listing that a step result of a browse job carries.
///
/// Only a successful step of a job of the matching kind stores a listing, so an agent cannot fill
/// the table through another job. A second write keeps the first (the agent sends a result twice).
pub async fn store_listing(
    tx: &mut TenantTx<'_>,
    job: &Job,
    output: Option<&step_result::Output>,
    now: OffsetDateTime,
) -> Result<(), restichub_db::DbError> {
    match (job.kind, output) {
        (JobKind::Snapshots, Some(step_result::Output::SnapshotList(list))) => {
            restore_repo::insert_listing(tx, job.id, "snapshots", &stored_snapshots(list), now)
                .await
        }
        (JobKind::Ls, Some(step_result::Output::LsPage(page))) => {
            restore_repo::insert_listing(tx, job.id, "ls", &stored_page(page), now).await
        }
        _ => Ok(()),
    }
}

/// The state of a browse job as the API shows it.
pub fn state_of(job: &Job, has_listing: bool) -> BrowseState {
    match job.status {
        JobStatus::Queued | JobStatus::Sent | JobStatus::Running => BrowseState::Pending,
        JobStatus::Succeeded if has_listing => BrowseState::Done,
        JobStatus::Succeeded | JobStatus::Failed | JobStatus::Expired | JobStatus::DeadLetter => {
            BrowseState::Failed
        }
    }
}

/// The stable error code of a browse job that ended without a listing (NFR-DOC-04).
pub fn error_code_of(job: &Job) -> Option<String> {
    if let Some(message) = &job.result_message {
        return Some(message.clone());
    }
    match job.status {
        JobStatus::Expired => Some("expired".to_owned()),
        JobStatus::Succeeded => Some("listing_missing".to_owned()),
        _ => job.result_code.map(|code| {
            serde_json::to_value(code)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "failed".to_owned())
        }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use restichub_proto::agent::v1::{BackupSummary, Snapshot};

    use super::*;

    #[test]
    fn ac_rs_01_a_listing_from_an_agent_is_bounded_and_cleaned() {
        let list = SnapshotList {
            snapshots: (0..MAX_SNAPSHOTS + 5)
                .map(|index| Snapshot {
                    id: format!("{index:064}"),
                    hostname: "host\u{1b}[31m".to_owned(),
                    tags: vec!["t".repeat(10_000)],
                    paths: (0..500).map(|path| format!("/p{path}")).collect(),
                    summary: Some(BackupSummary {
                        total_files_processed: u64::MAX,
                        ..BackupSummary::default()
                    }),
                    ..Snapshot::default()
                })
                .collect(),
            truncated: false,
        };
        let stored = stored_snapshots(&list);
        assert!(stored.truncated);
        assert_eq!(stored.snapshots.len(), MAX_SNAPSHOTS);
        let first = &stored.snapshots[0];
        assert_eq!(first.hostname, "host[31m");
        assert_eq!(first.tags[0].chars().count(), MAX_TEXT_CHARS);
        assert_eq!(first.paths.len(), MAX_LIST_ITEMS);
        assert_eq!(first.total_files, Some(i64::MAX));
        assert_eq!(first.time, None);
    }

    #[test]
    fn ac_rs_02_a_page_from_an_agent_is_bounded() {
        let page = LsPage {
            nodes: (0..6_000)
                .map(|index| Node {
                    name: format!("file-{index}\n"),
                    size: -4,
                    r#type: NodeType::Dir as i32,
                    ..Node::default()
                })
                .collect(),
            last: true,
        };
        let stored = stored_page(&page);
        assert_eq!(stored.nodes.len(), 5_000);
        assert!(!stored.last, "a cut page is not the last page");
        assert_eq!(stored.nodes[1].name, "file-1");
        assert_eq!(stored.nodes[1].size, 0);
        assert_eq!(stored.nodes[1].kind, NodeKind::Dir);
    }
}
