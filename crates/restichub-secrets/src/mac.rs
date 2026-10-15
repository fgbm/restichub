//! HMAC-SHA256 and the keys that the control plane derives from the KEK (ADR-0041).

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::kek::Kek;

const BLOCK: usize = 64;

/// HMAC-SHA256 of `message` under `key` (RFC 2104).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = Zeroizing::new([0_u8; BLOCK]);
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.iter().map(|byte| byte ^ 0x36).collect::<Vec<u8>>());
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block.iter().map(|byte| byte ^ 0x5c).collect::<Vec<u8>>());
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// Derives a 32-byte key for one purpose from the KEK.
///
/// The `context` names the purpose and the scope, so two contexts give unrelated keys. A KEK
/// rotation changes the derived key.
pub fn derive_key(kek: &Kek, context: &[u8]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(hmac_sha256(kek.bytes(), context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KEK_LEN;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn hmac_sha256_matches_the_rfc_4231_vectors() {
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn derive_key_separates_contexts() {
        let mut bytes = [7_u8; KEK_LEN];
        let kek = Kek::from_bytes(&mut bytes);
        assert_ne!(*derive_key(&kek, b"a"), *derive_key(&kek, b"b"));
        assert_eq!(*derive_key(&kek, b"a"), *derive_key(&kek, b"a"));
    }
}
