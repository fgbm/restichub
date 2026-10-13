use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE};

/// Byte forms that a canary value can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Raw,
    Utf16Le,
    Base64Standard,
    Base64Url,
    HexLower,
    HexUpper,
    PercentUpper,
    PercentLower,
    JsonEscaped,
}

impl Encoding {
    /// Text for the output line.
    pub fn label(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Utf16Le => "utf16le",
            Self::Base64Standard => "base64",
            Self::Base64Url => "base64url",
            Self::HexLower => "hex-lower",
            Self::HexUpper => "hex-upper",
            Self::PercentUpper => "percent-upper",
            Self::PercentLower => "percent-lower",
            Self::JsonEscaped => "json",
        }
    }
}

/// Bytes of one encoding and the base64 group alignment. The bytes never reach the output.
pub(crate) struct Pattern {
    pub(crate) encoding: Encoding,
    pub(crate) alignment: u8,
    pub(crate) bytes: Vec<u8>,
}

/// Builds the byte patterns of every encoding for one canary value.
///
/// Two encodings that give the same bytes collapse to one pattern. For the marker `CANARY-<uuid>`
/// the percent-encoding and the JSON escaping are the raw bytes, so only the raw pattern remains.
pub(crate) fn patterns(canary: &[u8]) -> Vec<Pattern> {
    let mut out: Vec<Pattern> = Vec::new();
    push(&mut out, Encoding::Raw, 0, canary.to_vec());
    push(&mut out, Encoding::Utf16Le, 0, utf16le(canary));
    for alignment in 0..3u8 {
        for candidate in base64_aligned(canary, usize::from(alignment), false) {
            push(&mut out, Encoding::Base64Standard, alignment, candidate);
        }
    }
    for alignment in 0..3u8 {
        for candidate in base64_aligned(canary, usize::from(alignment), true) {
            push(&mut out, Encoding::Base64Url, alignment, candidate);
        }
    }
    push(&mut out, Encoding::HexLower, 0, hex(canary, false));
    push(&mut out, Encoding::HexUpper, 0, hex(canary, true));
    push(&mut out, Encoding::PercentUpper, 0, percent(canary, true));
    push(&mut out, Encoding::PercentLower, 0, percent(canary, false));
    push(&mut out, Encoding::JsonEscaped, 0, json_escape(canary));
    out
}

fn push(out: &mut Vec<Pattern>, encoding: Encoding, alignment: u8, bytes: Vec<u8>) {
    if bytes.is_empty() || out.iter().any(|pattern| pattern.bytes == bytes) {
        return;
    }
    out.push(Pattern {
        encoding,
        alignment,
        bytes,
    });
}

fn utf16le(canary: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(canary.len() * 2);
    for &byte in canary {
        out.push(byte);
        out.push(0);
    }
    out
}

/// Builds the base64 substrings that hold the canary at one group alignment.
///
/// A canary starts at byte position 0, 1 or 2 inside a base64 group of three bytes. The bits of the
/// byte before the canary are unknown, so the function builds one candidate for each bit
/// combination at the group boundary. Only the base64 characters that depend on the canary alone
/// enter a candidate, so the last character is absent when the canary length is not a multiple of
/// three. This limit keeps the search exact: a partial character also depends on the bytes after the
/// canary.
fn base64_aligned(canary: &[u8], alignment: usize, url_safe: bool) -> Vec<Vec<u8>> {
    if canary.is_empty() {
        return Vec::new();
    }
    let filler_chars = alignment * 8 / 6;
    let total_bits = (alignment + canary.len()) * 8;
    let end_chars = total_bits / 6;
    if end_chars <= filler_chars {
        return Vec::new();
    }
    let boundary_bits = alignment * 8 % 6;
    let variants = 1usize << boundary_bits;
    let mut out: Vec<Vec<u8>> = Vec::new();
    for variant in 0..variants {
        let mut input = vec![0u8; alignment];
        if alignment > 0 {
            input[alignment - 1] = variant as u8;
        }
        input.extend_from_slice(canary);
        let encoded = if url_safe {
            URL_SAFE.encode(&input)
        } else {
            STANDARD.encode(&input)
        };
        let candidate = encoded.as_bytes()[filler_chars..end_chars].to_vec();
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn hex(canary: &[u8], upper: bool) -> Vec<u8> {
    const LOWER: &[u8; 16] = b"0123456789abcdef";
    const UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let table = if upper { UPPER } else { LOWER };
    let mut out = Vec::with_capacity(canary.len() * 2);
    for &byte in canary {
        out.push(table[usize::from(byte >> 4)]);
        out.push(table[usize::from(byte & 0x0f)]);
    }
    out
}

fn percent(canary: &[u8], upper: bool) -> Vec<u8> {
    const LOWER: &[u8; 16] = b"0123456789abcdef";
    const UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let table = if upper { UPPER } else { LOWER };
    let mut out = Vec::with_capacity(canary.len() * 3);
    for &byte in canary {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte);
        } else {
            out.push(b'%');
            out.push(table[usize::from(byte >> 4)]);
            out.push(table[usize::from(byte & 0x0f)]);
        }
    }
    out
}

fn json_escape(canary: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(canary.len());
    for &byte in canary {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x00..=0x1f => out.extend_from_slice(format!("\\u{byte:04x}").as_bytes()),
            _ => out.push(byte),
        }
    }
    out
}
