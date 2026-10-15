//! The restic option and version matrix (AC-AG-09).

use std::fmt;

use crate::target::ResticVersion;

/// A restic option that ResticHub can set (AC-AG-09).
///
/// The variants cover the options of the MVP step types. `RestoreOwnershipByName`
/// and `CheckSnapshotFilter` belong to step types of the restore and snapshot
/// flows. They stay in the matrix so that the control plane validates every
/// option of the product in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResticOption {
    UseFsSnapshot,
    OneFileSystem,
    ExcludeCaches,
    /// `backup --exclude-cloud-files` (Windows only, restic 0.17).
    ExcludeCloudFiles,
    LimitUpload,
    LimitDownload,
    RetryLock,
    ReadDataSubset,
    KeepLast,
    KeepHourly,
    KeepDaily,
    KeepWeekly,
    KeepMonthly,
    KeepYearly,
    KeepWithin,
    KeepTags,
    MaxUnused,
    MaxRepackSize,
    /// `restore --ownership-by-name` (restic 0.19.0).
    RestoreOwnershipByName,
    /// Snapshot filters of `check` (restic 0.19.0).
    CheckSnapshotFilter,
}

impl ResticOption {
    /// CLI spelling, for example `--retry-lock`.
    #[must_use]
    pub fn flag(self) -> &'static str {
        match self {
            Self::UseFsSnapshot => "--use-fs-snapshot",
            Self::OneFileSystem => "--one-file-system",
            Self::ExcludeCaches => "--exclude-caches",
            Self::ExcludeCloudFiles => "--exclude-cloud-files",
            Self::LimitUpload => "--limit-upload",
            Self::LimitDownload => "--limit-download",
            Self::RetryLock => "--retry-lock",
            Self::ReadDataSubset => "--read-data-subset",
            Self::KeepLast => "--keep-last",
            Self::KeepHourly => "--keep-hourly",
            Self::KeepDaily => "--keep-daily",
            Self::KeepWeekly => "--keep-weekly",
            Self::KeepMonthly => "--keep-monthly",
            Self::KeepYearly => "--keep-yearly",
            Self::KeepWithin => "--keep-within",
            Self::KeepTags => "--keep-tag",
            Self::MaxUnused => "--max-unused",
            Self::MaxRepackSize => "--max-repack-size",
            Self::RestoreOwnershipByName => "--ownership-by-name",
            Self::CheckSnapshotFilter => "--host",
        }
    }

    /// The oldest restic version that has the option (AC-AG-09).
    ///
    /// An option that exists at or before [`ResticVersion::MIN_SUPPORTED`] has
    /// the floor as its minimum. The table is data; update it when restic adds
    /// an option.
    #[must_use]
    pub fn minimum_version(self) -> ResticVersion {
        match self {
            Self::RestoreOwnershipByName | Self::CheckSnapshotFilter => ResticVersion {
                major: 0,
                minor: 19,
                patch: 0,
            },
            _ => ResticVersion::MIN_SUPPORTED,
        }
    }
}

impl fmt::Display for ResticOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.flag())
    }
}

#[cfg(test)]
mod tests {
    use super::ResticOption;
    use crate::check_restic_version;
    use crate::error::{CompileError, FieldPath};
    use crate::target::ResticVersion;

    #[test]
    fn ac_ag_09_option_needing_newer_restic_is_incompatible() {
        let installed = ResticVersion::MIN_SUPPORTED;
        let result = check_restic_version(ResticOption::RestoreOwnershipByName, installed);
        assert_eq!(
            result,
            Err(CompileError::IncompatibleOption {
                field: FieldPath::new("--ownership-by-name"),
                option: ResticOption::RestoreOwnershipByName,
                required: ResticVersion {
                    major: 0,
                    minor: 19,
                    patch: 0
                },
                installed,
            })
        );
    }

    #[test]
    fn option_of_the_supported_version_is_compatible() {
        assert!(
            check_restic_version(ResticOption::RetryLock, ResticVersion::MIN_SUPPORTED).is_ok()
        );
        assert!(
            check_restic_version(
                ResticOption::RestoreOwnershipByName,
                ResticVersion {
                    major: 0,
                    minor: 19,
                    patch: 1
                }
            )
            .is_ok()
        );
    }
}
