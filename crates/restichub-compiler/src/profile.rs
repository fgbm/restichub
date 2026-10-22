//! A profile: the input of the compiler (FR-PF-01).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::target::OperatingSystem;

/// A profile: the paths, the excludes, the tags, the options, the scripts and the retention
/// (FR-PF-01). The repository belongs to the plan, so it is not part of the profile (ADR-0047).
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub paths: ProfilePaths,
    pub excludes: Vec<String>,
    pub exclude_files: Vec<String>,
    pub tags: Vec<String>,
    /// Marks the snapshots of this profile (FR-RP-08). The compiler adds it to the backup and to
    /// the filter of the `forget` step. A user tag may not use its reserved prefix.
    pub profile_tag: Option<String>,
    pub options: BackupOptions,
    pub limits: JobLimits,
    /// Code that the control plane sends (FR-HK-09, ADR-0037).
    pub scripts: Vec<crate::script::ScriptRef>,
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
    /// Low asks the agent to lower the process and I/O priority of the compiled steps (FR-AG-15).
    /// The maintenance job of a repository builds its own `prune` and `check` steps, which do not
    /// carry it.
    pub priority: Priority,
}

/// The process and I/O priority that a profile asks for (FR-AG-15).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Priority {
    /// The agent keeps the priority of the operating system default.
    #[default]
    Normal,
    /// Lower process and I/O priority (AC-AG-14).
    Low,
}

/// The time limits of a job (ADR-0039). Zero means the default of the stall timeout and no
/// maximum duration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobLimits {
    /// Seconds without progress after which the agent stops a step.
    pub stall_timeout_seconds: u32,
    /// Seconds after which the agent stops the job, whatever its progress.
    pub max_duration_seconds: u32,
}

/// Position of a script in the job (FR-HK-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
