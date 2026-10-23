//! Log lines about VSS snapshots of a backup step (FR-AG-08).
//!
//! With `--json` restic prints no VSS message: its "creating VSS snapshot" and "successfully
//! created snapshot" lines exist in text mode only. restic reports a failure, and only as an `error`
//! message on the standard error. A backup that ends with a summary and has no such error made a
//! snapshot of each volume that it read, because restic makes the snapshot of a volume when it
//! reads the first path on it.

use restichub_proto::agent::v1::{Step, step};
use serde_json::Value;

/// The text that opens the restic error for a volume without a snapshot.
const FAILURE_PREFIX: &str = "failed to create snapshot for [";

/// The volumes of a backup step that asks for a file system snapshot.
pub(super) struct VssReport {
    /// The drive-letter volumes of the paths, in the form of `C:\`.
    volumes: Vec<String>,
    /// The volumes that restic named in a failure, in lower case.
    failed: Vec<String>,
}

impl VssReport {
    /// The report of a step, or `None` when the step does not ask for a snapshot.
    pub(super) fn for_step(step: &Step) -> Option<Self> {
        let Some(step::Kind::Backup(backup)) = step.kind.as_ref() else {
            return None;
        };
        if !backup.use_fs_snapshot {
            return None;
        }
        let mut volumes: Vec<String> = Vec::new();
        for path in &backup.paths {
            if let Some(volume) = volume_of(path)
                && !volumes
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(&volume))
            {
                volumes.push(volume);
            }
        }
        Some(Self {
            volumes,
            failed: Vec::new(),
        })
    }

    /// Reads one line of the standard error. Returns the log line for a snapshot that restic could
    /// not make.
    pub(super) fn failure(&mut self, line: &str, truncated: bool) -> Option<String> {
        if truncated || !line.starts_with('{') {
            return None;
        }
        let message: Value = serde_json::from_str(line).ok()?;
        if message.get("message_type").and_then(Value::as_str) != Some("error") {
            return None;
        }
        let text = message
            .get("error")?
            .get("message")?
            .as_str()?
            .strip_prefix(FAILURE_PREFIX)?;
        let (volume, reason) = text.split_once("]: ")?;
        self.failed.push(volume.to_ascii_lowercase());
        Some(format!(
            "VSS snapshot for {volume} failed: {reason}; restic reads the files of this volume without a snapshot"
        ))
    }

    /// The log lines for the volumes with a snapshot, once the backup ended with a summary.
    pub(super) fn made(&self) -> Vec<String> {
        self.volumes
            .iter()
            .filter(|volume| {
                !self
                    .failed
                    .iter()
                    .any(|failed| failed.eq_ignore_ascii_case(volume))
            })
            .map(|volume| format!("VSS snapshot made for {volume} (restic reported no VSS error)"))
            .collect()
    }
}

/// The volume of a path with a drive letter, in the form of `C:\`. A network path has none: restic
/// does not snapshot it.
fn volume_of(path: &str) -> Option<String> {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let mut chars = path.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':')).then(|| format!(r"{}:\", letter.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::{BackupStep, Step, step};

    use super::VssReport;

    fn report(paths: &[&str], use_fs_snapshot: bool) -> Option<VssReport> {
        VssReport::for_step(&Step {
            step_id: "backup".to_owned(),
            kind: Some(step::Kind::Backup(BackupStep {
                paths: paths.iter().map(|path| (*path).to_owned()).collect(),
                use_fs_snapshot,
                ..BackupStep::default()
            })),
            ..Step::default()
        })
    }

    const FAILURE: &str = r#"{"message_type":"error","error":{"message":"failed to create snapshot for [c:\\]: VSS error: access denied"},"during":"archival","item":"c:\\"}"#;

    #[test]
    fn fr_ag_08_a_step_without_a_snapshot_has_no_report() {
        assert!(report(&[r"C:\data"], false).is_none());
    }

    #[test]
    fn fr_ag_08_the_volumes_come_from_the_drive_letters_once() {
        let report = report(
            &[r"c:\data", r"C:\more", r"D:\x", r"\\server\share\y"],
            true,
        )
        .expect("a report");
        assert_eq!(
            report.made(),
            [
                r"VSS snapshot made for C:\ (restic reported no VSS error)",
                r"VSS snapshot made for D:\ (restic reported no VSS error)",
            ]
        );
    }

    #[test]
    fn fr_ag_08_a_failure_line_names_the_volume_and_the_reason() {
        let mut report = report(&[r"C:\data", r"D:\x"], true).expect("a report");
        assert_eq!(
            report.failure(FAILURE, false).as_deref(),
            Some(
                r"VSS snapshot for c:\ failed: VSS error: access denied; restic reads the files of this volume without a snapshot"
            )
        );
        assert_eq!(
            report.made(),
            [r"VSS snapshot made for D:\ (restic reported no VSS error)"]
        );
    }

    #[test]
    fn fr_ag_08_other_stderr_lines_are_not_a_failure() {
        let mut report = report(&[r"C:\data"], true).expect("a report");
        for line in [
            "plain text",
            r#"{"message_type":"error","error":{"message":"open x: access denied"},"during":"archival","item":"x"}"#,
            r#"{"message_type":"exit_error","message":"failed","code":1}"#,
        ] {
            assert_eq!(report.failure(line, false), None, "{line}");
        }
        assert_eq!(report.failure(FAILURE, true), None);
    }
}
