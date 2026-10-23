//! Log lines about VSS snapshots of a backup step (FR-AG-08).
//!
//! With `--json` restic prints no VSS message: its "creating VSS snapshot" and "successfully
//! created snapshot" lines exist in text mode only. restic reports a failure, and only as an `error`
//! message on the standard error. A backup that ends with a summary and has no such error made a
//! snapshot of each volume that it read, because restic makes the snapshot of a volume when it
//! reads the first path on it. restic drops a path that is missing or unreadable before it makes
//! any snapshot, and says so in plain text on the standard error.
//!
//! restic gives the same `error` message for a failed snapshot and for another error on the root of
//! a volume, so only the opening text of the error tells them apart. The agent claims a snapshot
//! for neither.

use restichub_proto::agent::v1::{Step, step};
use serde_json::Value;

/// The text that opens the restic error for a volume without a snapshot.
const FAILURE_PREFIX: &str = "failed to create snapshot for [";

/// The texts that restic prints after a path that it leaves out of the backup.
const SKIP_SUFFIXES: [&str; 2] = [" does not exist, skipping", " cannot be accessed, skipping"];

struct Volume {
    /// In the form of `C:\`.
    name: String,
    paths: Vec<String>,
}

/// The volumes of a backup step that asks for a file system snapshot.
pub(super) struct VssReport {
    volumes: Vec<Volume>,
    /// The paths that restic left out of the backup.
    skipped: Vec<String>,
    /// The volumes with an error of restic on their root, in the form of `C:\`.
    reported: Vec<String>,
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
        let mut volumes: Vec<Volume> = Vec::new();
        for path in &backup.paths {
            let Some(name) = volume_of(path) else {
                continue;
            };
            match volumes
                .iter_mut()
                .find(|known| known.name.eq_ignore_ascii_case(&name))
            {
                Some(volume) => volume.paths.push(path.clone()),
                None => volumes.push(Volume {
                    name,
                    paths: vec![path.clone()],
                }),
            }
        }
        Some(Self {
            volumes,
            skipped: Vec::new(),
            reported: Vec::new(),
        })
    }

    /// Reads one line of the standard error. Returns the log line for a volume whose root has an
    /// error of restic.
    pub(super) fn observe(&mut self, line: &str, truncated: bool) -> Option<String> {
        if truncated {
            return None;
        }
        if !line.starts_with('{') {
            self.note_skip(line);
            return None;
        }
        let message: Value = serde_json::from_str(line).ok()?;
        if message.get("message_type").and_then(Value::as_str) != Some("error") {
            return None;
        }
        // restic names the volume of a failed snapshot as the item of the error.
        let volume = drive_root(message.get("item")?.as_str()?)?;
        let text = message.get("error")?.get("message")?.as_str()?;
        let line = match text.strip_prefix(FAILURE_PREFIX) {
            Some(rest) => {
                let reason = rest.split_once("]: ").map_or(text, |(_, reason)| reason);
                format!(
                    "VSS snapshot for {volume} failed: {reason}; restic reads the files of this volume without a snapshot"
                )
            }
            None => format!("VSS snapshot for {volume} not confirmed: {text}"),
        };
        self.reported.push(volume);
        Some(line)
    }

    fn note_skip(&mut self, line: &str) {
        let line = line.trim_end();
        for suffix in SKIP_SUFFIXES {
            if let Some(path) = line.strip_suffix(suffix) {
                self.skipped.push(path.to_owned());
            }
        }
    }

    /// The log lines for the volumes with a snapshot, once the backup ended with a summary.
    pub(super) fn made(&self) -> Vec<String> {
        self.volumes
            .iter()
            .filter(|volume| {
                let reported = self
                    .reported
                    .iter()
                    .any(|reported| reported.eq_ignore_ascii_case(&volume.name));
                let read = volume.paths.iter().any(|path| !self.skipped.contains(path));
                !reported && read
            })
            .map(|volume| {
                format!(
                    "VSS snapshot made for {} (restic reported no VSS error)",
                    volume.name
                )
            })
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

/// The drive root `C:\` of an item that is exactly a drive root, in upper case.
fn drive_root(item: &str) -> Option<String> {
    let mut chars = item.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':') && chars.next() == Some('\\') && chars.next().is_none())
        .then(|| format!(r"{}:\", letter.to_ascii_uppercase()))
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
            report.observe(FAILURE, false).as_deref(),
            Some(
                r"VSS snapshot for C:\ failed: VSS error: access denied; restic reads the files of this volume without a snapshot"
            )
        );
        assert_eq!(
            report.made(),
            [r"VSS snapshot made for D:\ (restic reported no VSS error)"]
        );
    }

    #[test]
    fn fr_ag_08_another_error_for_a_drive_root_is_not_confirmed() {
        let mut report = report(&[r"C:\data", r"D:\x"], true).expect("a report");
        let line = r#"{"message_type":"error","error":{"message":"something new"},"item":"d:\\"}"#;
        assert_eq!(
            report.observe(line, false).as_deref(),
            Some(r"VSS snapshot for D:\ not confirmed: something new")
        );
        assert_eq!(
            report.made(),
            [r"VSS snapshot made for C:\ (restic reported no VSS error)"]
        );
    }

    #[test]
    fn fr_ag_08_a_metadata_error_of_restic_0_17_2_is_not_a_vss_failure() {
        let mut report = report(&[r"C:\data"], true).expect("a report");
        let line = r#"{"message_type":"error","error":{"message":"incomplete metadata for C:\\: get EA failed while opening file handle for path \\\\?\\GLOBALROOT\\Device\\HarddiskVolumeShadowCopy12\\, with: Access is denied."},"during":"archival","item":"C:\\"}"#;
        let logged = report.observe(line, false).expect("a log line");
        assert!(
            logged
                .starts_with(r"VSS snapshot for C:\ not confirmed: incomplete metadata for C:\: "),
            "{logged}"
        );
        assert!(!logged.contains("without a snapshot"), "{logged}");
        assert!(report.made().is_empty());
    }

    #[test]
    fn fr_ag_08_a_volume_with_every_path_left_out_has_no_snapshot() {
        let mut report = report(&[r"C:\data", r"D:\more", r"D:\other"], true).expect("a report");
        assert_eq!(
            report.observe(r"D:\more does not exist, skipping", false),
            None
        );
        assert_eq!(
            report.made(),
            [
                r"VSS snapshot made for C:\ (restic reported no VSS error)",
                r"VSS snapshot made for D:\ (restic reported no VSS error)",
            ]
        );
        assert_eq!(
            report.observe(r"D:\other cannot be accessed, skipping", false),
            None
        );
        assert_eq!(
            report.made(),
            [r"VSS snapshot made for C:\ (restic reported no VSS error)"]
        );
    }

    #[test]
    fn fr_ag_08_other_stderr_lines_are_not_a_failure() {
        let mut report = report(&[r"C:\data"], true).expect("a report");
        for line in [
            "plain text",
            r#"{"message_type":"error","error":{"message":"open x: access denied"},"during":"archival","item":"x"}"#,
            r#"{"message_type":"error","error":{"message":"open c:\\data: access denied"},"during":"archival","item":"c:\\data"}"#,
            r#"{"message_type":"exit_error","message":"failed","code":1}"#,
        ] {
            assert_eq!(report.observe(line, false), None, "{line}");
        }
        assert_eq!(report.observe(FAILURE, true), None);
        assert_eq!(
            report.observe(r"C:\data does not exist, skipping", true),
            None
        );
        assert_eq!(report.made().len(), 1);
    }
}
