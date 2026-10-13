//! Canary secret scan (NFR-SEC-04, NFR-SEC-05).
//!
//! The scan looks for a canary marker in each location where a secret must not occur. It searches
//! the byte forms that one value can take: raw bytes, UTF-16LE, base64, hexadecimal,
//! percent-encoding and JSON escaping. A finding names the target, the byte offset, the line, the
//! encoding and a masked snippet. The tool never prints the canary value.
//!
//! The library part is reusable, so a test scans a database dump or a log buffer with the same
//! search as the `restichub-canary-scan` binary.

mod encoding;
mod scan;

#[cfg(test)]
mod tests;

pub use encoding::Encoding;
pub use scan::{Canary, Finding, Scanner, cmdline_paths};
