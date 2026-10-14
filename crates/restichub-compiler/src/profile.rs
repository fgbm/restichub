//! A profile: the input of the compiler (FR-PF-01).

use std::collections::BTreeMap;

use crate::target::OperatingSystem;

/// A profile: the repository, the paths, the excludes, the tags, the options,
/// the hooks and the retention (FR-PF-01).
#[derive(Debug, Clone, Default)]
pub struct Profile {
    /// Identifier of the repository in the control plane.
    pub repository_id: String,
    pub paths: ProfilePaths,
    pub excludes: Vec<String>,
    pub exclude_files: Vec<String>,
    pub tags: Vec<String>,
    /// Marks the snapshots of this profile (FR-RP-08). The compiler adds it to the backup and to
    /// the filter of the `forget` step. A user tag may not use its reserved prefix.
    pub profile_tag: Option<String>,
    pub options: BackupOptions,
    pub hooks: Vec<HookRef>,
    /// Policy that produces the `forget` and `prune` steps.
    pub retention: Option<Retention>,
    /// Policy that produces the verification step.
    pub check: Option<CheckOptions>,
}

/// Paths to back up. Paths for each operating system are separate (FR-PF-01).
#[derive(Debug, Clone, Default)]
pub struct ProfilePaths {
    /// Paths for every operating system.
    pub all: Vec<String>,
    /// Extra paths for one operating system.
    pub by_os: BTreeMap<OperatingSystem, Vec<String>>,
}

impl ProfilePaths {
    /// The paths of one operating system: `all` followed by `by_os[os]`.
    #[must_use]
    pub fn for_os(&self, os: OperatingSystem) -> Vec<&String> {
        let mut paths: Vec<&String> = self.all.iter().collect();
        if let Some(extra) = self.by_os.get(&os) {
            paths.extend(extra);
        }
        paths
    }
}

/// Backup options (FR-PF-01).
#[derive(Debug, Clone, Default)]
pub struct BackupOptions {
    /// Use a file system snapshot. VSS on Windows only (FR-PF-01).
    pub use_fs_snapshot: bool,
    pub one_file_system: bool,
    pub exclude_caches: bool,
    /// Upload limit in kibibytes each second. Zero means no limit (FR-AG-15).
    pub limit_upload_kib: u64,
    /// Download limit in kibibytes each second. Zero means no limit (FR-AG-15).
    pub limit_download_kib: u64,
}

/// A hook of the profile, by allow-list identifier (FR-HK-02, FR-HK-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRef {
    /// Identifier of the hook in the agent allow-list.
    pub hook_id: String,
    pub phase: HookPhase,
    pub parameters: Vec<HookParameter>,
}

/// One named hook parameter (FR-HK-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookParameter {
    pub name: String,
    pub value: String,
}

/// Position of a hook in the job (FR-HK-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPhase {
    Before,
    After,
    OnSuccess,
    OnError,
    Finally,
}

/// Policy of the profile (FR-PF-01).
#[derive(Debug, Clone, Default)]
pub struct Retention {
    pub keep_last: u32,
    pub keep_hourly: u32,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    pub keep_yearly: u32,
    /// Keep the snapshots younger than a duration, for example `30d`.
    pub keep_within: String,
    pub keep_tags: Vec<String>,
    pub group_by: Vec<GroupBy>,
    pub filter: SnapshotFilter,
}

/// Grouping key of the retention rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Host,
    Paths,
    Tags,
}

/// Snapshot filter of the retention rule.
#[derive(Debug, Clone, Default)]
pub struct SnapshotFilter {
    pub hosts: Vec<String>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
}

/// Controls the verification step (FR-PF-01).
#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    /// Portion to check, for example `10%` or `1/7`.
    pub read_data_subset: String,
}
