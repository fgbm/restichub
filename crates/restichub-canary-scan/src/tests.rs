use std::path::PathBuf;

use base64::Engine as _;

use crate::{Canary, Encoding, Scanner, cmdline_paths};

const MARKER: &str = "CANARY-7f3a91c2d4e5";

fn scanner(value: &str) -> Scanner {
    Scanner::new(&[Canary::parse(value)])
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "restichub-canary-scan-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn a_clean_input_has_no_findings() {
    let findings = scanner(MARKER).scan("target", b"the text holds no marker");
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn the_offset_and_the_line_are_reported() {
    let input = "first line\nsecond line holds CANARY-7f3a91c2d4e5 here\nthird";
    let findings = scanner(MARKER).scan("target", input.as_bytes());
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].offset, input.find(MARKER).unwrap() as u64);
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[0].encoding, Encoding::Raw);
}

#[test]
fn each_encoding_is_found() {
    // The slash, the space and the quote make the percent-encoding and the JSON escaping different
    // from the raw bytes.
    let value = "CANARY-7f3a/9 \"2";
    let scanner = scanner(value);
    let cases: [(&str, Vec<u8>, Option<Encoding>); 9] = [
        ("raw", value.as_bytes().to_vec(), Some(Encoding::Raw)),
        (
            "utf16le",
            utf16le(value.as_bytes()),
            Some(Encoding::Utf16Le),
        ),
        (
            "base64",
            base64::engine::general_purpose::STANDARD
                .encode(value)
                .into_bytes(),
            None,
        ),
        (
            "base64url",
            base64::engine::general_purpose::URL_SAFE
                .encode(value)
                .into_bytes(),
            None,
        ),
        (
            "hex-lower",
            hex(value.as_bytes(), false),
            Some(Encoding::HexLower),
        ),
        (
            "hex-upper",
            hex(value.as_bytes(), true),
            Some(Encoding::HexUpper),
        ),
        (
            "percent-upper",
            percent(value.as_bytes(), true),
            Some(Encoding::PercentUpper),
        ),
        (
            "percent-lower",
            percent(value.as_bytes(), false),
            Some(Encoding::PercentLower),
        ),
        (
            "json",
            json_escape(value.as_bytes()),
            Some(Encoding::JsonEscaped),
        ),
    ];
    for (name, encoded, expected) in cases {
        let findings = scanner.scan("target", &encoded);
        assert!(!findings.is_empty(), "the {name} form is not found");
        if let Some(expected) = expected {
            assert!(
                findings.iter().any(|finding| finding.encoding == expected),
                "the {name} form gives {findings:?}"
            );
        } else {
            assert!(
                findings.iter().any(|finding| matches!(
                    finding.encoding,
                    Encoding::Base64Standard | Encoding::Base64Url
                )),
                "the {name} form gives {findings:?}"
            );
        }
    }
}

#[test]
fn a_canary_inside_a_base64_blob_is_found() {
    for offset in 0..3usize {
        let mut plaintext = vec![b'Z'; offset];
        plaintext.extend_from_slice(MARKER.as_bytes());
        plaintext.extend_from_slice(b"trailing bytes");
        let blob = base64::engine::general_purpose::STANDARD.encode(&plaintext);
        let findings = scanner(MARKER).scan("target", blob.as_bytes());
        assert!(
            !findings.is_empty(),
            "the base64 group offset {offset} is missed"
        );
        assert!(findings.iter().all(|finding| matches!(
            finding.encoding,
            Encoding::Base64Standard | Encoding::Base64Url
        )));
    }
}

#[test]
fn the_snippet_never_holds_the_canary() {
    let input = format!("{MARKER}{MARKER} and again {MARKER}");
    let findings = scanner(MARKER).scan("target", input.as_bytes());
    assert!(findings.len() >= 3, "{findings:?}");
    for finding in &findings {
        assert!(
            !finding.snippet.contains(MARKER),
            "the snippet holds the canary: {}",
            finding.snippet
        );
    }
}

#[test]
fn a_large_input_streams_across_the_chunk_boundary() {
    let value = "CANARY-STREAM-1234567890";
    let size = (1 << 20) * 2 + 7;
    let mut data = vec![b'.'; size];
    let at = (1 << 20) - 4;
    data[at..at + value.len()].copy_from_slice(value.as_bytes());
    let findings = scanner(value).scan_reader("target", &data[..]).unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].offset, at as u64);
    assert_eq!(findings[0].encoding, Encoding::Raw);
}

#[test]
fn ac_ag_03_a_canary_in_the_agent_directory_is_found() {
    let dir = temp_dir("agent-found");
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("state/journal"), "job 12 CANARY-DIR-42 done").unwrap();
    let findings = scanner("CANARY-DIR-42").scan_directory(&dir).unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn ac_ag_03_a_clean_agent_directory_finds_no_canary() {
    let dir = temp_dir("agent-clean");
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("state/journal"), "job 12 done").unwrap();
    std::fs::write(dir.join("config.toml"), "server = \"cp.example\"").unwrap();
    let findings = scanner("CANARY-DIR-42").scan_directory(&dir).unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn ac_ag_04_a_canary_in_a_process_argument_is_found() {
    let content = b"/usr/bin/restic\x00--repo\x00/storage\x00--password\x00CANARY-ARG-9\x00";
    let findings = scanner("CANARY-ARG-9").scan("cmdline", content);
    assert_eq!(findings.len(), 1, "{findings:?}");
}

#[test]
fn ac_ag_04_the_cmdline_paths_select_the_process_directories() {
    let dir = temp_dir("proc");
    std::fs::create_dir_all(dir.join("123")).unwrap();
    std::fs::create_dir_all(dir.join("self")).unwrap();
    std::fs::write(dir.join("123/cmdline"), b"restic\x00job").unwrap();
    std::fs::write(dir.join("uptime"), b"12345").unwrap();
    let paths = cmdline_paths(&dir).unwrap();
    assert_eq!(paths, vec![dir.join("123/cmdline")]);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn utf16le(canary: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &byte in canary {
        out.push(byte);
        out.push(0);
    }
    out
}

fn hex(canary: &[u8], upper: bool) -> Vec<u8> {
    const LOWER: &[u8; 16] = b"0123456789abcdef";
    const UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let table = if upper { UPPER } else { LOWER };
    let mut out = Vec::new();
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
    let mut out = Vec::new();
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
    let mut out = Vec::new();
    for &byte in canary {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            _ => out.push(byte),
        }
    }
    out
}
