//! A strict OpenPGP verifier for the restic release signature (ADR-0038, threat T20).
//!
//! The verifier reads one public key from an armored key block and checks one detached signature.
//! It accepts only a version 4 RSA key and a version 4 binary-document signature with SHA-2, which
//! is what the restic release process makes. Anything else is an error, and a key counts only when
//! its fingerprint equals the pinned one.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ring::signature::{
    RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_2048_8192_SHA384, RSA_PKCS1_2048_8192_SHA512,
    RsaParameters, UnparsedPublicKey,
};
use sha1::{Digest as _, Sha1};

const TAG_SIGNATURE: u8 = 2;
const TAG_PUBLIC_KEY: u8 = 6;
const ALGORITHM_RSA: u8 = 1;
const SIGNATURE_BINARY_DOCUMENT: u8 = 0x00;
const SUBPACKET_ISSUER_FINGERPRINT: u8 = 33;

/// The signing key of the restic releases, pinned by its fingerprint.
#[derive(Debug, Clone)]
pub struct PublicKey {
    fingerprint: [u8; 20],
    modulus_len: usize,
    rsa_der: Vec<u8>,
}

impl PublicKey {
    /// Reads the key with the pinned fingerprint from an armored key block.
    ///
    /// The block may hold other keys. They are ignored.
    ///
    /// # Errors
    ///
    /// Returns an error when the armor is bad or the block holds no usable key with the
    /// fingerprint.
    pub fn from_armored(armored: &str, fingerprint_hex: &str) -> Result<Self> {
        let wanted = parse_fingerprint(fingerprint_hex)?;
        for packet in packets(&dearmor(armored)?)? {
            if packet.tag != TAG_PUBLIC_KEY {
                continue;
            }
            let fingerprint = fingerprint_of(packet.body);
            if fingerprint != wanted {
                continue;
            }
            return Self::from_packet(packet.body, fingerprint);
        }
        bail!("the key block holds no key with the fingerprint {fingerprint_hex}")
    }

    fn from_packet(body: &[u8], fingerprint: [u8; 20]) -> Result<Self> {
        let mut reader = Reader::new(body);
        if reader.u8()? != 4 {
            bail!("the key is not a version 4 key");
        }
        reader.take(4)?;
        if reader.u8()? != ALGORITHM_RSA {
            bail!("the key is not an RSA key");
        }
        let modulus = reader.mpi()?;
        let exponent = reader.mpi()?;
        let rsa_der = rsa_public_key_der(modulus, exponent);
        Ok(Self {
            fingerprint,
            modulus_len: modulus.len(),
            rsa_der,
        })
    }
}

/// Checks that an armored detached signature over `data` comes from the pinned key.
///
/// # Errors
///
/// Returns an error when the signature is malformed, uses an unsupported algorithm, names another
/// key, or does not match `data`.
pub fn verify_detached(key: &PublicKey, data: &[u8], armored_signature: &str) -> Result<()> {
    let binary = dearmor(armored_signature)?;
    let mut found = None;
    for packet in packets(&binary)? {
        if packet.tag != TAG_SIGNATURE {
            bail!("the signature holds a packet that is not a signature");
        }
        if found.replace(packet).is_some() {
            bail!("the signature holds more than one signature packet");
        }
    }
    let packet = found.context("the signature holds no signature packet")?;
    let signature = Signature::parse(packet.body)?;
    if signature.issuer_fingerprint != Some(key.fingerprint) {
        bail!("the signature does not name the pinned release key");
    }
    if signature.modulus_bytes.len() > key.modulus_len {
        bail!("the signature is longer than the key");
    }
    // The signature integer drops its leading zeros, and the verifier needs the full key length.
    let mut padded = vec![0_u8; key.modulus_len - signature.modulus_bytes.len()];
    padded.extend_from_slice(signature.modulus_bytes);

    let mut message = Vec::with_capacity(data.len() + signature.hashed_prefix.len() + 6);
    message.extend_from_slice(data);
    message.extend_from_slice(signature.hashed_prefix);
    let trailer_len =
        u32::try_from(signature.hashed_prefix.len()).context("the signature header is too long")?;
    message.extend_from_slice(&[4, 0xFF]);
    message.extend_from_slice(&trailer_len.to_be_bytes());

    UnparsedPublicKey::new(signature.algorithm, &key.rsa_der)
        .verify(&message, &padded)
        .map_err(|error| anyhow::anyhow!("the signature does not match the data: {error}"))
}

struct Signature<'a> {
    algorithm: &'static RsaParameters,
    hashed_prefix: &'a [u8],
    issuer_fingerprint: Option<[u8; 20]>,
    modulus_bytes: &'a [u8],
}

impl<'a> Signature<'a> {
    fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);
        if reader.u8()? != 4 {
            bail!("the signature is not a version 4 signature");
        }
        if reader.u8()? != SIGNATURE_BINARY_DOCUMENT {
            bail!("the signature is not a signature of a binary document");
        }
        if reader.u8()? != ALGORITHM_RSA {
            bail!("the signature is not an RSA signature");
        }
        let algorithm = match reader.u8()? {
            8 => &RSA_PKCS1_2048_8192_SHA256,
            9 => &RSA_PKCS1_2048_8192_SHA384,
            10 => &RSA_PKCS1_2048_8192_SHA512,
            other => bail!("the signature hash algorithm {other} is not supported"),
        };
        let hashed_len = usize::from(reader.u16()?);
        let hashed = reader.take(hashed_len)?;
        // The hashed bytes of the header are exactly the first six bytes plus the hashed area.
        let hashed_prefix = &body[..6 + hashed_len];
        let issuer_fingerprint = issuer_fingerprint(hashed)?;
        let unhashed_len = usize::from(reader.u16()?);
        reader.take(unhashed_len)?;
        reader.take(2)?;
        let modulus_bytes = reader.mpi()?;
        if !reader.is_empty() {
            bail!("the signature holds trailing bytes");
        }
        Ok(Self {
            algorithm,
            hashed_prefix,
            issuer_fingerprint,
            modulus_bytes,
        })
    }
}

fn issuer_fingerprint(mut area: &[u8]) -> Result<Option<[u8; 20]>> {
    let mut found = None;
    while !area.is_empty() {
        let mut reader = Reader::new(area);
        let length = match reader.u8()? {
            first @ 0..=191 => usize::from(first),
            first @ 192..=254 => {
                ((usize::from(first) - 192) << 8) + usize::from(reader.u8()?) + 192
            }
            255 => usize::try_from(reader.u32()?).context("a subpacket is too long")?,
        };
        let consumed = area.len() - reader.rest().len();
        if length == 0 || area.len() < consumed + length {
            bail!("a signature subpacket is cut short");
        }
        let subpacket = &area[consumed..consumed + length];
        // Bit 7 marks a critical subpacket.
        if subpacket[0] & 0x7F == SUBPACKET_ISSUER_FINGERPRINT {
            let [4, rest @ ..] = &subpacket[1..] else {
                bail!("the issuer fingerprint is not a version 4 fingerprint");
            };
            let fingerprint: [u8; 20] = rest
                .try_into()
                .map_err(|_| anyhow::anyhow!("the issuer fingerprint has a bad length"))?;
            found = Some(fingerprint);
        }
        area = &area[consumed + length..];
    }
    Ok(found)
}

struct Packet<'a> {
    tag: u8,
    body: &'a [u8],
}

fn packets(mut data: &[u8]) -> Result<Vec<Packet<'_>>> {
    let mut list = Vec::new();
    while !data.is_empty() {
        let mut reader = Reader::new(data);
        let header = reader.u8()?;
        if header & 0x80 == 0 {
            bail!("the data is not an OpenPGP packet");
        }
        let (tag, length) = if header & 0x40 != 0 {
            let length = match reader.u8()? {
                first @ 0..=191 => usize::from(first),
                first @ 192..=223 => {
                    ((usize::from(first) - 192) << 8) + usize::from(reader.u8()?) + 192
                }
                255 => usize::try_from(reader.u32()?).context("a packet is too long")?,
                _ => bail!("a packet of partial length is not supported"),
            };
            (header & 0x3F, length)
        } else {
            let length = match header & 0x03 {
                0 => usize::from(reader.u8()?),
                1 => usize::from(reader.u16()?),
                2 => usize::try_from(reader.u32()?).context("a packet is too long")?,
                _ => bail!("a packet of indeterminate length is not supported"),
            };
            ((header >> 2) & 0x0F, length)
        };
        let body = reader.take(length)?;
        list.push(Packet { tag, body });
        data = reader.rest();
    }
    Ok(list)
}

fn fingerprint_of(key_body: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update([0x99]);
    // A key packet body is far below 64 KiB, and an oversized one never matches the pin.
    hasher.update(
        u16::try_from(key_body.len())
            .unwrap_or(u16::MAX)
            .to_be_bytes(),
    );
    hasher.update(key_body);
    hasher.finalize().into()
}

fn parse_fingerprint(hex: &str) -> Result<[u8; 20]> {
    let hex = hex.trim();
    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("`{hex}` is not a 40 digit fingerprint");
    }
    let mut out = [0_u8; 20];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .context("the fingerprint is not hexadecimal")?;
    }
    Ok(out)
}

fn dearmor(armored: &str) -> Result<Vec<u8>> {
    let mut lines = armored.lines().map(str::trim);
    let begin = lines
        .by_ref()
        .find(|line| line.starts_with("-----BEGIN PGP"))
        .context("the text holds no PGP armor")?;
    if begin.starts_with("-----BEGIN PGP SIGNED MESSAGE") {
        bail!("a cleartext signed message is not supported");
    }
    // The armor headers end at the first empty line.
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
    }
    let mut encoded = String::new();
    let mut closed = false;
    for line in lines {
        // The checksum line starts with `=`, and base64 padding never starts a line.
        if line.starts_with('=') {
            continue;
        }
        if line.starts_with("-----END PGP") {
            closed = true;
            break;
        }
        encoded.push_str(line);
    }
    if !closed {
        bail!("the PGP armor has no end line");
    }
    STANDARD
        .decode(encoded)
        .context("the PGP armor is not valid base64")
}

fn rsa_public_key_der(modulus: &[u8], exponent: &[u8]) -> Vec<u8> {
    let mut body = der_integer(modulus);
    body.extend(der_integer(exponent));
    let mut out = vec![0x30];
    out.extend(der_length(body.len()));
    out.extend(body);
    out
}

fn der_integer(value: &[u8]) -> Vec<u8> {
    let trimmed = &value[value.iter().take_while(|byte| **byte == 0).count()..];
    let mut content = Vec::with_capacity(trimmed.len() + 1);
    if trimmed.first().is_none_or(|byte| byte & 0x80 != 0) {
        content.push(0);
    }
    content.extend_from_slice(trimmed);
    let mut out = vec![0x02];
    out.extend(der_length(content.len()));
    out.extend(content);
    out
}

fn der_length(length: usize) -> Vec<u8> {
    if length < 0x80 {
        return vec![u8::try_from(length).unwrap_or(0)];
    }
    let bytes: Vec<u8> = length
        .to_be_bytes()
        .into_iter()
        .skip_while(|byte| *byte == 0)
        .collect();
    let mut out = vec![0x80 | u8::try_from(bytes.len()).unwrap_or(0)];
    out.extend(bytes);
    out
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn rest(&self) -> &'a [u8] {
        self.data
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.data.len() < count {
            bail!("the OpenPGP data is cut short");
        }
        let (head, tail) = self.data.split_at(count);
        self.data = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Reads a multiprecision integer and returns its bytes without the leading zero bytes.
    fn mpi(&mut self) -> Result<&'a [u8]> {
        let bits = usize::from(self.u16()?);
        let bytes = self.take(bits.div_ceil(8))?;
        Ok(&bytes[bytes.iter().take_while(|byte| **byte == 0).count()..])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    const KEY: &str = include_str!("../../../packaging/restic-release-key.asc");
    const FINGERPRINT: &str = "CF8F18F2844575973F79D4E191A6868BD3F7A907";
    const SUMS: &[u8] = include_bytes!("../testdata/restic-0.19.1/SHA256SUMS");
    const SIGNATURE: &str = include_str!("../testdata/restic-0.19.1/SHA256SUMS.asc");

    fn key() -> PublicKey {
        PublicKey::from_armored(KEY, FINGERPRINT).expect("the pinned key must parse")
    }

    #[test]
    fn nfr_sec_06_the_real_restic_signature_verifies() {
        verify_detached(&key(), SUMS, SIGNATURE).expect("the release signature must verify");
    }

    #[test]
    fn nfr_sec_06_a_changed_file_fails() {
        let mut changed = SUMS.to_vec();
        changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
        assert!(verify_detached(&key(), &changed, SIGNATURE).is_err());
    }

    #[test]
    fn nfr_sec_06_a_key_with_another_fingerprint_is_not_found() {
        let error = PublicKey::from_armored(KEY, &"A".repeat(40)).expect_err("no such key");
        assert!(error.to_string().contains("no key with the fingerprint"));
    }

    #[test]
    fn nfr_sec_06_a_signature_that_names_another_key_fails() {
        let mut other = key();
        other.fingerprint[0] ^= 1;
        let error = verify_detached(&other, SUMS, SIGNATURE).expect_err("another key");
        assert!(error.to_string().contains("does not name the pinned"));
    }

    #[test]
    fn nfr_sec_06_a_flipped_signature_byte_fails() {
        let binary = dearmor(SIGNATURE).expect("armor");
        let mut broken = binary.clone();
        let last = broken.len() - 1;
        broken[last] ^= 1;
        let armored = format!(
            "-----BEGIN PGP SIGNATURE-----\n\n{}\n-----END PGP SIGNATURE-----\n",
            STANDARD.encode(broken)
        );
        assert!(verify_detached(&key(), SUMS, &armored).is_err());
    }

    #[test]
    fn nfr_sec_06_garbage_and_cleartext_are_refused() {
        assert!(verify_detached(&key(), SUMS, "not armor").is_err());
        assert!(
            verify_detached(
                &key(),
                SUMS,
                "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA512\n\nx\n-----END PGP SIGNATURE-----"
            )
            .is_err()
        );
    }

    #[test]
    fn nfr_sec_06_the_fingerprint_matches_the_packaging_pin() {
        let pin = include_str!("../../../packaging/restic.env");
        assert!(
            pin.contains(&format!("RESTIC_RELEASE_KEY_FINGERPRINT={FINGERPRINT}")),
            "the embedded pin differs from packaging/restic.env"
        );
    }
}
