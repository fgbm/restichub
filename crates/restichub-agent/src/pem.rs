//! PEM encoding and decoding of the stored certificates.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Wraps a DER value in a PEM block with the given label.
#[must_use]
pub fn encode(label: &str, der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let mut text = format!("-----BEGIN {label}-----\n");
    for chunk in body.as_bytes().chunks(64) {
        text.push_str(&String::from_utf8_lossy(chunk));
        text.push('\n');
    }
    text.push_str(&format!("-----END {label}-----\n"));
    text
}

/// Returns the DER of every PEM block in the text, in order.
///
/// # Errors
///
/// Fails when a block is not base64.
pub fn decode_all(text: &str) -> Result<Vec<Vec<u8>>> {
    let mut blocks = Vec::new();
    let mut body = String::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN ") {
            inside = true;
            body.clear();
            continue;
        }
        if line.starts_with("-----END ") {
            if inside {
                blocks.push(
                    STANDARD
                        .decode(body.as_bytes())
                        .context("a PEM block is not base64")?,
                );
            }
            inside = false;
            continue;
        }
        if inside {
            body.push_str(line);
        }
    }
    if inside {
        bail!("the PEM text holds an unterminated block");
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_round_trips() {
        let der = vec![1_u8, 2, 3, 4, 5];
        let text = encode("CERTIFICATE", &der);
        assert!(text.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert_eq!(decode_all(&text).expect("decode"), vec![der]);
    }

    #[test]
    fn two_blocks_keep_their_order() {
        let text = format!(
            "{}{}",
            encode("CERTIFICATE", &[1]),
            encode("CERTIFICATE", &[2])
        );
        assert_eq!(decode_all(&text).expect("decode"), vec![vec![1], vec![2]]);
    }

    #[test]
    fn a_long_body_wraps_at_64_columns() {
        let text = encode("CERTIFICATE", &[0_u8; 60]);
        for line in text.lines().skip(1) {
            if line.starts_with("-----END") {
                break;
            }
            assert!(line.len() <= 64, "line too long: {line}");
        }
    }
}
