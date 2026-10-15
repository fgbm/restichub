//! The HMAC-SHA256 signature of a webhook payload (ADR-0035).
//!
//! The receiver recomputes `HMAC-SHA256(secret, timestamp + "." + body)` and compares it with the
//! header `X-ResticHub-Signature: sha256=<hex>`. The timestamp is the header
//! `X-ResticHub-Timestamp`, in seconds since the Unix epoch; the receiver rejects an old one.

use sha2::{Digest, Sha256};

const BLOCK: usize = 64;

pub const SIGNATURE_HEADER: &str = "X-ResticHub-Signature";
pub const TIMESTAMP_HEADER: &str = "X-ResticHub-Timestamp";
pub const DELIVERY_HEADER: &str = "X-ResticHub-Delivery";

/// RFC 2104 with SHA-256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut block = [0_u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(data);
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// Header text `sha256=<hex>` over the timestamp, a dot and the body.
pub fn signature_header(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut data = Vec::with_capacity(body.len() + 21);
    data.extend_from_slice(timestamp.to_string().as_bytes());
    data.push(b'.');
    data.extend_from_slice(body);
    format!("sha256={}", hex(&hmac_sha256(secret, &data)))
}

/// Checks a signature header in constant time. A receiver, and the tests, use it.
pub fn verify(secret: &[u8], timestamp: i64, body: &[u8], header: &str) -> bool {
    let expected = signature_header(secret, timestamp, body);
    let (a, b) = (expected.as_bytes(), header.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0_u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-MN-06: RFC 4231 test case 1.
    #[test]
    fn ac_mn_06_hmac_known_answer() {
        let key = [0x0b_u8; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    /// AC-MN-06: RFC 4231 test case 6, a key longer than the block.
    #[test]
    fn ac_mn_06_hmac_long_key() {
        let key = [0xaa_u8; 131];
        let mac = hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            hex(&mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    /// AC-MN-06
    #[test]
    fn ac_mn_06_signature_verifies_and_rejects_a_change() {
        let header = signature_header(b"secret", 1_760_000_000, b"{\"a\":1}");
        assert!(verify(b"secret", 1_760_000_000, b"{\"a\":1}", &header));
        assert!(!verify(b"secret", 1_760_000_001, b"{\"a\":1}", &header));
        assert!(!verify(b"secret", 1_760_000_000, b"{\"a\":2}", &header));
        assert!(!verify(b"other", 1_760_000_000, b"{\"a\":1}", &header));
    }
}
