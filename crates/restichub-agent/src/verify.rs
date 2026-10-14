//! Validation of the enrollment response before the identity is stored (ADR-0026, FR-AG-04).
//!
//! `register` checks what the control plane returned before it writes anything: the leaf carries the
//! agent public key and the expected SAN URI, the leaf chains to the root in `ca_chain`, and the job
//! signing key set is valid. A failed check leaves the configuration directory untouched.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::VerifyingKey;
use prost::Message as _;
use restichub_proto::agent::v1::{KeyUpdate, SigningKeyUpdate};
use restichub_proto::signing::{EnrollmentKey, KeyRing, KeyUpdateError, verify_enrollment_keys};
use time::OffsetDateTime;
use x509_parser::prelude::{FromDer as _, GeneralName, X509Certificate};

use crate::enroll::{Enrolled, EnrollmentKeyBody};

/// The SAN URI that binds the certificate to one agent and one tenant (ADR-0026).
#[must_use]
pub fn san_uri(tenant_id: &str, agent_id: &str) -> String {
    format!("restichub://tenant/{tenant_id}/agent/{agent_id}")
}

/// Checks the issued certificate, its chain and the key set before anything is written (ADR-0026).
///
/// # Errors
///
/// Returns an error when the certificate does not carry the agent key or the expected SAN URI, when
/// the chain does not link the leaf to the root, or when the key set is not valid.
pub fn check_enrollment(
    agent_public_key: &[u8],
    enrolled: &Enrolled,
    now: OffsetDateTime,
) -> Result<KeyRing> {
    check_certificate(
        agent_public_key,
        &enrolled.agent_id,
        &enrolled.tenant_id,
        &enrolled.certificate_der,
        &enrolled.ca_chain_der,
        now,
    )?;
    verify_key_set(&enrolled.signing_keys, &enrolled.signing_keys_signature)
}

/// Verifies the delivered key set and returns a ring with the current and the next key (ADR-0026).
///
/// The current key signs the set, so the check proves that the set is self-consistent and detects a
/// partial tamper of the transport. The authenticity comes from the enrollment channel (threat
/// T13). The ring verifies a job that the current key signed at once.
///
/// # Errors
///
/// Returns an error when a key is not base64 or not a valid Ed25519 key, when the set holds no
/// current key, or when the signature does not verify.
pub fn verify_key_set(keys: &[EnrollmentKeyBody], signature: &str) -> Result<KeyRing> {
    let keys = decode_keys(keys)?;
    let signature = STANDARD
        .decode(signature)
        .context("the key set signature is not base64")?;
    let current = keys
        .iter()
        .find(|key| key.role == "current")
        .context("the key set holds no current key")?;
    let current_key = verifying_key(&current.public_key)?;
    verify_enrollment_keys(&keys, &signature, &current_key)
        .map_err(|error| anyhow::anyhow!("the key set signature is not valid: {error:?}"))?;
    let mut ring = KeyRing::new();
    for key in &keys {
        ring.insert(key.key_id.clone(), verifying_key(&key.public_key)?)
            .map_err(|error| anyhow::anyhow!("the key set is not valid: {error:?}"))?;
    }
    Ok(ring)
}

fn decode_keys(keys: &[EnrollmentKeyBody]) -> Result<Vec<EnrollmentKey>> {
    keys.iter()
        .map(|key| {
            Ok(EnrollmentKey {
                role: key.role.clone(),
                key_id: key.key_id.clone(),
                public_key: STANDARD
                    .decode(&key.public_key)
                    .context("a key is not base64")?,
            })
        })
        .collect()
}

/// Checks the leaf key, the leaf SAN and the chain to the root (ADR-0026).
///
/// # Errors
///
/// Returns an error when a certificate does not parse, when the key or the SAN does not match, when
/// a signature in the chain does not verify, or when a validity window is wrong.
pub fn check_certificate(
    agent_public_key: &[u8],
    agent_id: &str,
    tenant_id: &str,
    certificate_der: &[u8],
    ca_chain_der: &[Vec<u8>],
    now: OffsetDateTime,
) -> Result<()> {
    if agent_public_key.is_empty() {
        bail!("the agent public key is empty");
    }
    let (_, leaf) = X509Certificate::from_der(certificate_der)
        .map_err(|error| anyhow::anyhow!("the issued certificate does not parse: {error}"))?;
    if leaf.public_key().subject_public_key.data != agent_public_key {
        bail!("the issued certificate does not carry the agent public key");
    }
    let expected = san_uri(tenant_id, agent_id);
    let san = leaf
        .subject_alternative_name()
        .map_err(|error| {
            anyhow::anyhow!("the SAN of the issued certificate is not readable: {error}")
        })?
        .context("the issued certificate holds no SAN")?;
    let found = san
        .value
        .general_names
        .iter()
        .any(|name| matches!(name, GeneralName::URI(uri) if *uri == expected.as_str()));
    if !found {
        bail!("the issued certificate does not carry the SAN URI {expected}");
    }
    check_chain(&leaf, ca_chain_der, now)
}

fn check_chain(
    leaf: &X509Certificate<'_>,
    ca_chain_der: &[Vec<u8>],
    now: OffsetDateTime,
) -> Result<()> {
    if ca_chain_der.len() < 2 {
        bail!("the CA chain holds fewer than the issuing CA and the root");
    }
    let mut chain = Vec::with_capacity(ca_chain_der.len());
    for der in ca_chain_der {
        let (_, certificate) = X509Certificate::from_der(der)
            .map_err(|error| anyhow::anyhow!("a CA chain certificate does not parse: {error}"))?;
        chain.push(certificate);
    }

    // The leaf is an end entity, so it must not be a certificate authority (RFC 5280, ADR-0026).
    if let Some(constraints) = leaf
        .basic_constraints()
        .map_err(|error| anyhow::anyhow!("a basic constraint is not readable: {error}"))?
        && constraints.value.ca
    {
        bail!("the issued certificate is a certificate authority");
    }
    for (index, certificate) in chain.iter().enumerate() {
        let constraints = certificate
            .basic_constraints()
            .map_err(|error| anyhow::anyhow!("a basic constraint is not readable: {error}"))?
            .context("a chain certificate holds no basic constraints")?;
        if !constraints.value.ca {
            bail!("a chain certificate is not a certificate authority");
        }
        // The issuing CA signs the leaf only, so its path length is 0. The root CA signs the
        // issuing CA, so its path length is at least 1 (ADR-0026).
        let limit = constraints.value.path_len_constraint;
        if index == 0 && limit.is_some_and(|limit| limit != 0) {
            bail!("the issuing CA has a path length other than 0");
        }
        if index + 1 == chain.len() && limit.is_some_and(|limit| limit < 1) {
            bail!("the root CA cannot sign the issuing CA");
        }
    }

    valid_at(leaf, now)?;
    leaf.verify_signature(Some(chain[0].public_key()))
        .map_err(|error| anyhow::anyhow!("the leaf is not signed by the issuing CA: {error}"))?;
    if leaf.validity().not_after.to_datetime() > chain[0].validity().not_after.to_datetime() {
        bail!("the leaf certificate outlives its issuing CA");
    }
    for index in 0..chain.len() - 1 {
        valid_at(&chain[index], now)?;
        if chain[index].validity().not_after.to_datetime()
            > chain[index + 1].validity().not_after.to_datetime()
        {
            bail!("a CA certificate outlives its issuer");
        }
        chain[index]
            .verify_signature(Some(chain[index + 1].public_key()))
            .map_err(|error| {
                anyhow::anyhow!("a CA certificate is not signed by its issuer: {error}")
            })?;
    }
    let root = chain.last().context("the CA chain is empty")?;
    valid_at(root, now)?;
    root.verify_signature(None)
        .map_err(|error| anyhow::anyhow!("the root certificate is not self-signed: {error}"))?;
    Ok(())
}

fn valid_at(certificate: &X509Certificate<'_>, now: OffsetDateTime) -> Result<()> {
    let validity = certificate.validity();
    if now < validity.not_before.to_datetime() || now > validity.not_after.to_datetime() {
        bail!("a chain certificate is not valid at the current time");
    }
    Ok(())
}

/// Adopts one signed job signing key update into the ring (FR-PR-11, ADR-0026).
///
/// The signature is verified whenever the agent can check it: it holds the signer in the ring, or
/// the update introduces the signer itself. At the first enrollment the agent holds no job signing
/// key, and the update is signed by the current key while it introduces the next key. The trust then
/// comes from the enrollment channel only: an HTTPS connection verified to the control plane with a
/// single-use token (ADR-0026, threat T13, trust on first use). Every later update is verified
/// against the ring. The function refuses a signature that the agent can check.
///
/// # Errors
///
/// Returns an error when the update does not decode, names two different signers, names no key, or
/// carries a public key or a signature that does not verify.
pub fn adopt_key_update(ring: &mut KeyRing, bytes: &[u8]) -> Result<()> {
    let envelope = SigningKeyUpdate::decode(bytes).context("the key update does not decode")?;
    let inner = KeyUpdate::decode(envelope.key_update.as_slice())
        .context("the inner key update does not decode")?;
    if inner.signer_key_id != envelope.signer_key_id {
        bail!("the key update names two different signers");
    }
    if inner.key_id.is_empty() {
        bail!("the key update names no key");
    }
    let key = verifying_key(&inner.public_key)?;
    // The probe ring holds the introduced key, so a self-signed update is verified. An update signed
    // by a key from an earlier key set gives `UnknownSigner`, which the first use accepts.
    let mut probe = ring.clone();
    probe
        .insert(inner.key_id.clone(), key)
        .map_err(|error| anyhow::anyhow!("the key update is not valid: {error:?}"))?;
    match probe.apply_key_update(&envelope) {
        Ok(()) | Err(KeyUpdateError::UnknownSigner) => {}
        Err(error) => bail!("the key update is not valid: {error:?}"),
    }
    ring.insert(inner.key_id, key)
        .map_err(|error| anyhow::anyhow!("the key update is not valid: {error:?}"))?;
    Ok(())
}

fn verifying_key(bytes: &[u8]) -> Result<VerifyingKey> {
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("the job signing public key is not 32 bytes"))?;
    VerifyingKey::from_bytes(&array)
        .context("the job signing public key is not a valid Ed25519 key")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair,
        PKCS_ECDSA_P256_SHA256, SanType,
    };
    use restichub_proto::agent::v1::KeyUpdate;
    use restichub_proto::signing::sign_key_update;

    use super::*;

    const TENANT: &str = "0192f1e0-0000-7000-8000-0000000000a1";
    const AGENT: &str = "0192f1e0-0000-7000-8000-0000000000b2";

    fn agent_key() -> KeyPair {
        KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the agent key")
    }

    fn issued(agent_key: &KeyPair, san: &str) -> (Vec<u8>, Vec<Vec<u8>>) {
        let root_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("root key");
        let mut root_params = CertificateParams::new(Vec::new()).expect("root parameters");
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        root_params
            .distinguished_name
            .push(DnType::CommonName, "test-root");
        let root = CertifiedIssuer::self_signed(root_params, root_key).expect("root");

        let issuing_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("issuing key");
        let mut issuing_params = CertificateParams::new(Vec::new()).expect("issuing parameters");
        issuing_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        issuing_params
            .distinguished_name
            .push(DnType::CommonName, "test-issuing");
        let issuing =
            CertifiedIssuer::signed_by(issuing_params, issuing_key, &root).expect("issuing");

        let mut leaf_params = CertificateParams::new(Vec::new()).expect("leaf parameters");
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "agent");
        leaf_params
            .subject_alt_names
            .push(SanType::URI(san.to_owned().try_into().expect("SAN URI")));
        let leaf = leaf_params
            .signed_by(agent_key, &issuing)
            .expect("leaf certificate");
        (
            leaf.der().to_vec(),
            vec![issuing.der().to_vec(), root.der().to_vec()],
        )
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    #[test]
    fn a_valid_certificate_passes() {
        let key = agent_key();
        let (leaf, chain) = issued(&key, &san_uri(TENANT, AGENT));
        check_certificate(key.public_key_raw(), AGENT, TENANT, &leaf, &chain, now())
            .expect("the certificate must pass");
    }

    #[test]
    fn a_certificate_for_another_key_is_refused() {
        let other = agent_key();
        let (leaf, chain) = issued(&other, &san_uri(TENANT, AGENT));
        let key = agent_key();
        let error = check_certificate(key.public_key_raw(), AGENT, TENANT, &leaf, &chain, now())
            .expect_err("a certificate for another key must fail");
        assert!(error.to_string().contains("agent public key"));
    }

    #[test]
    fn a_certificate_with_another_san_is_refused() {
        let key = agent_key();
        let (leaf, chain) = issued(&key, &san_uri(TENANT, "another-agent"));
        let error = check_certificate(key.public_key_raw(), AGENT, TENANT, &leaf, &chain, now())
            .expect_err("a certificate with another SAN must fail");
        assert!(error.to_string().contains("SAN URI"));
    }

    #[test]
    fn a_chain_without_the_root_is_refused() {
        let key = agent_key();
        let (leaf, chain) = issued(&key, &san_uri(TENANT, AGENT));
        let truncated = vec![chain[0].clone()];
        assert!(
            check_certificate(
                key.public_key_raw(),
                AGENT,
                TENANT,
                &leaf,
                &truncated,
                now()
            )
            .is_err()
        );
    }

    #[test]
    fn a_first_use_key_update_is_adopted() {
        // The real control plane signs the update with the current key and introduces the next key.
        let current = signing_key(1);
        let update = KeyUpdate {
            key_id: "next".to_owned(),
            public_key: signing_key(2).verifying_key().to_bytes().to_vec(),
            signer_key_id: "current".to_owned(),
            issued_at: None,
            revoked_key_ids: Vec::new(),
        };
        let envelope = sign_key_update(&update, &current);
        let mut ring = KeyRing::new();
        adopt_key_update(&mut ring, &envelope.encode_to_vec()).expect("first use must be accepted");
        assert!(ring.get("next").is_some());
    }

    #[test]
    fn a_bad_key_update_signature_is_refused() {
        // A self-signed update, so the agent holds the signing key and must verify the signature.
        let signer = signing_key(3);
        let update = KeyUpdate {
            key_id: "key".to_owned(),
            public_key: signer.verifying_key().to_bytes().to_vec(),
            signer_key_id: "key".to_owned(),
            issued_at: None,
            revoked_key_ids: Vec::new(),
        };
        let mut envelope = sign_key_update(&update, &signer);
        envelope.signature[0] ^= 0xff;
        let mut ring = KeyRing::new();
        let error = adopt_key_update(&mut ring, &envelope.encode_to_vec())
            .expect_err("a bad signature must be refused");
        assert!(error.to_string().contains("not valid"), "{error}");
        assert!(ring.get("key").is_none());
    }

    fn signing_key(seed: u8) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
    }
}
