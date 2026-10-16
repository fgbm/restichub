//! The size-rotated log file of the Windows service.
//!
//! Each file has a sequence number, `agent-000001.log` and up, and the highest number is the
//! current file. A rotation opens the next number and never renames a file, so a reader that holds
//! a file open on Windows cannot stop it. An old file that a reader holds stays until a later
//! rotation removes it.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

const PREFIX: &str = "agent-";
const SUFFIX: &str = ".log";

pub struct RotatingFile {
    dir: PathBuf,
    max_bytes: u64,
    max_files: usize,
    file: File,
    sequence: u64,
    size: u64,
}

impl RotatingFile {
    /// Continues the newest file of `dir`, or starts a new one when it is full.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be listed or the file cannot be opened.
    pub fn open(dir: &Path, max_bytes: u64, max_files: usize) -> std::io::Result<Self> {
        let newest = sequences(dir)?.into_iter().max();
        let (file, sequence, size) = match newest {
            Some(sequence) => {
                let path = dir.join(file_name(sequence));
                let size = std::fs::symlink_metadata(&path)?.len();
                if size < max_bytes {
                    (OpenOptions::new().append(true).open(&path)?, sequence, size)
                } else {
                    (create(dir, sequence + 1)?, sequence + 1, 0)
                }
            }
            None => (create(dir, 1)?, 1, 0),
        };
        let file = Self {
            dir: dir.to_path_buf(),
            max_bytes,
            max_files,
            file,
            sequence,
            size,
        };
        file.prune();
        Ok(file)
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    #[must_use]
    pub fn current(&self) -> PathBuf {
        self.dir.join(file_name(self.sequence))
    }

    /// Appends one event. An event that does not fit into the current file starts the next one;
    /// an event larger than a whole file still goes into one file.
    ///
    /// # Errors
    ///
    /// Returns an error when the next file cannot be created or the write fails.
    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let length = bytes.len() as u64;
        if self.size > 0 && self.size.saturating_add(length) > self.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(bytes)?;
        self.size = self.size.saturating_add(length);
        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        let next = self.sequence + 1;
        self.file = create(&self.dir, next)?;
        self.sequence = next;
        self.size = 0;
        self.prune();
        Ok(())
    }

    /// Removes the oldest files above the limit. A file that cannot be removed now is tried again
    /// at the next rotation.
    fn prune(&self) {
        let Ok(mut sequences) = sequences(&self.dir) else {
            return;
        };
        sequences.sort_unstable();
        let excess = sequences.len().saturating_sub(self.max_files);
        for sequence in &sequences[..excess] {
            let _ = std::fs::remove_file(self.dir.join(file_name(*sequence)));
        }
    }
}

/// A new file never opens an existing path, so a planted link cannot redirect the log.
fn create(dir: &Path, sequence: u64) -> std::io::Result<File> {
    OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(dir.join(file_name(sequence)))
}

fn file_name(sequence: u64) -> String {
    format!("{PREFIX}{sequence:06}{SUFFIX}")
}

fn sequence_of(name: &str) -> Option<u64> {
    let digits = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The sequence numbers of the log files in `dir`. Other names stay as they are.
fn sequences(dir: &Path) -> std::io::Result<Vec<u64>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(sequence) = entry.file_name().to_str().and_then(sequence_of) {
            found.push(sequence);
        }
    }
    Ok(found)
}
