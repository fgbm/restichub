use std::fs::{File, read_dir};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::encoding::{Encoding, Pattern, patterns};

/// Value that must not occur in a target.
pub struct Canary {
    pub(crate) bytes: Vec<u8>,
}

impl Canary {
    /// Turns a text value into a canary.
    pub fn parse(value: &str) -> Self {
        Self {
            bytes: value.as_bytes().to_vec(),
        }
    }
}

impl std::fmt::Debug for Canary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Canary([redacted])")
    }
}

/// One location where an encoded canary occurs.
#[derive(Debug)]
pub struct Finding {
    pub target: String,
    pub offset: u64,
    pub line: u64,
    pub encoding: Encoding,
    pub alignment: u8,
    pub snippet: String,
}

/// Searches targets for the encoded forms of the canary values.
pub struct Scanner {
    patterns: Vec<Pattern>,
    overlap: usize,
}

/// The read size. A multi-GB dump never enters memory as one block.
const CHUNK: usize = 1 << 20;
/// The number of context bytes on each side of a match in the masked snippet.
const CONTEXT: usize = 32;

impl Scanner {
    /// Builds a scanner for one or more canary values.
    pub fn new(canaries: &[Canary]) -> Self {
        let mut list = Vec::new();
        for canary in canaries {
            list.extend(patterns(&canary.bytes));
        }
        let longest = list.iter().map(|p| p.bytes.len()).max().unwrap_or(0);
        Self {
            patterns: list,
            overlap: longest.saturating_sub(1),
        }
    }

    /// Reports each match in a byte slice.
    pub fn scan(&self, target: &str, haystack: &[u8]) -> Vec<Finding> {
        let mut findings = Vec::new();
        self.search(haystack, 0, 1, haystack.len(), target, &mut findings);
        findings.sort_by_key(|finding| finding.offset);
        findings
    }

    /// Reads a source in chunks and reports each match.
    ///
    /// The function keeps the last `overlap` bytes between two reads, so a pattern that crosses a
    /// chunk boundary is still found.
    pub fn scan_reader<R: Read>(&self, target: &str, mut reader: R) -> io::Result<Vec<Finding>> {
        let mut findings = Vec::new();
        let mut buffer: Vec<u8> = Vec::with_capacity(CHUNK + self.overlap);
        let mut chunk = vec![0u8; CHUNK];
        let mut base: u64 = 0;
        let mut first_line: u64 = 1;
        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                self.search(
                    &buffer,
                    base,
                    first_line,
                    buffer.len(),
                    target,
                    &mut findings,
                );
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            let consumed = buffer.len().saturating_sub(self.overlap);
            if consumed == 0 {
                continue;
            }
            self.search(&buffer, base, first_line, consumed, target, &mut findings);
            first_line += count_newlines(&buffer[..consumed]);
            base += consumed as u64;
            buffer.drain(..consumed);
        }
        findings.sort_by_key(|finding| finding.offset);
        Ok(findings)
    }

    /// Reads one file and reports each match.
    pub fn scan_file(&self, path: &Path) -> io::Result<Vec<Finding>> {
        let file = File::open(path).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", path.display()))
        })?;
        self.scan_reader(&path.to_string_lossy(), BufReader::new(file))
    }

    /// Walks a directory tree and reports each match.
    pub fn scan_directory(&self, path: &Path) -> io::Result<Vec<Finding>> {
        let mut findings = Vec::new();
        self.walk(path, &mut findings)?;
        Ok(findings)
    }

    fn walk(&self, path: &Path, findings: &mut Vec<Finding>) -> io::Result<()> {
        for entry in read_dir(path)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let child = entry.path();
            if file_type.is_dir() {
                self.walk(&child, findings)?;
            } else if file_type.is_file() {
                findings.extend(self.scan_file(&child)?);
            }
        }
        Ok(())
    }

    fn search(
        &self,
        buffer: &[u8],
        base: u64,
        first_line: u64,
        limit: usize,
        target: &str,
        findings: &mut Vec<Finding>,
    ) {
        let mut seen: Vec<(usize, usize)> = Vec::new();
        for pattern in &self.patterns {
            for start in find_all(buffer, &pattern.bytes, limit) {
                let key = (start, pattern.bytes.len());
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                findings.push(Finding {
                    target: target.to_owned(),
                    offset: base + start as u64,
                    line: first_line + count_newlines(&buffer[..start]),
                    encoding: pattern.encoding,
                    alignment: pattern.alignment,
                    snippet: masked_snippet(buffer, start, pattern.bytes.len(), &self.patterns),
                });
            }
        }
    }
}

/// Lists the process command-line files under a root, for `/proc` on Linux.
pub fn cmdline_paths(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(pid) = name.to_str() else {
            continue;
        };
        if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let path = entry.path().join("cmdline");
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn find_all(haystack: &[u8], needle: &[u8], limit: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let length = needle.len();
    if length == 0 || haystack.len() < length {
        return out;
    }
    let last = haystack.len() - length;
    let mut index = 0;
    while index <= last && index < limit {
        if haystack[index] == needle[0] && &haystack[index..index + length] == needle {
            out.push(index);
        }
        index += 1;
    }
    out
}

fn count_newlines(bytes: &[u8]) -> u64 {
    bytes.iter().filter(|&&byte| byte == b'\n').count() as u64
}

fn masked_snippet(buffer: &[u8], start: usize, length: usize, patterns: &[Pattern]) -> String {
    let window_start = start.saturating_sub(CONTEXT);
    let window_end = (start + length + CONTEXT).min(buffer.len());
    let mut out = String::new();
    out.push_str(&String::from_utf8_lossy(&buffer[window_start..start]));
    out.push_str("[REDACTED]");
    out.push_str(&String::from_utf8_lossy(
        &buffer[start + length..window_end],
    ));
    for pattern in patterns {
        let Ok(text) = std::str::from_utf8(&pattern.bytes) else {
            continue;
        };
        if !text.is_empty() && out.contains(text) {
            out = out.replace(text, "[REDACTED]");
        }
    }
    out
}
