//! The certificate rotation of the agent (ADR-0026, FR-ID-03).
//!
//! The agent asks for a new certificate over its open stream at two thirds of the certificate
//! lifetime. The request carries the public key of a new key pair, so a rotation also renews the
//! key. The agent checks the answer against its pinned root and its own identity, stores it with
//! the owner-only rules and tells the control plane. The stream goes on with the old certificate;
//! the new one serves the next connection.
//!
//! The rotation is part of the connection upkeep (FR-AG-02): it starts no job and runs no
//! schedule of backups (ADR-0002).

use anyhow::{Context, Result, bail};
use restichub_proto::agent::v1::CertificateIssued;
use time::OffsetDateTime;
use x509_parser::prelude::{FromDer as _, GeneralName, X509Certificate};

use crate::identity::{GeneratedKey, Rotated, StoredIdentity, generate_key_and_csr};

/// The rotation starts after this share of the lifetime has passed (FR-ID-03).
const ROTATION_NUMERATOR: i32 = 2;
const ROTATION_DENOMINATOR: i32 = 3;

/// The time at which the agent asks for a new certificate: two thirds of the lifetime.
#[must_use]
pub fn rotation_due(not_before: OffsetDateTime, not_after: OffsetDateTime) -> OffsetDateTime {
    not_before + (not_after - not_before) * ROTATION_NUMERATOR / ROTATION_DENOMINATOR
}

/// A request that waits for its answer. The new private key stays in memory until the certificate
/// arrives; it reaches the disk only through [`StoredIdentity::install_rotation`].
pub struct PendingRotation {
    key: GeneratedKey,
}

impl std::fmt::Debug for PendingRotation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingRotation")
            .finish_non_exhaustive()
    }
}

/// Makes a new key pair and the certificate signing request for it.
///
/// # Errors
///
/// Returns an error when the key or the request cannot be generated.
pub fn begin() -> Result<(PendingRotation, Vec<u8>)> {
    let (key, csr_der) = generate_key_and_csr()?;
    Ok((PendingRotation { key }, csr_der))
}

/// Checks the issued certificate and returns the files to install.
///
/// The certificate must carry the identity of this agent, the new public key and a validity that
/// has not ended. The chain must lead from the certificate over the issuing CA to the root that
/// the agent pinned at enrollment; the chain never replaces the pinned root.
///
/// # Errors
///
/// Returns an error when one of the checks fails. The agent then keeps its old identity.
pub fn accept(
    identity: &StoredIdentity,
    pending: &PendingRotation,
    issued: &CertificateIssued,
    now: OffsetDateTime,
) -> Result<Rotated> {
    let (_, leaf) = X509Certificate::from_der(&issued.certificate_der)
        .map_err(|error| anyhow::anyhow!("the issued certificate is not valid: {error}"))?;
    let expected = format!(
        "restichub://tenant/{}/agent/{}",
        identity.tenant_id, identity.agent_id
    );
    let uri = leaf
        .subject_alternative_name()
        .ok()
        .flatten()
        .and_then(|san| {
            san.value.general_names.iter().find_map(|name| match name {
                GeneralName::URI(uri) => Some((*uri).to_owned()),
                _ => None,
            })
        })
        .context("the issued certificate holds no identity URI")?;
    if uri != expected {
        bail!("the issued certificate names another identity");
    }
    if leaf.public_key().subject_public_key.data.as_ref() != pending.key.public_key.as_slice() {
        bail!("the issued certificate does not hold the requested public key");
    }
    let validity = leaf.validity();
    if now >= validity.not_after.to_datetime() {
        bail!("the issued certificate is already expired");
    }

    let pinned_root = identity
        .ca_chain_der
        .last()
        .context("the identity holds no pinned root")?;
    let Some((root_der, issuers)) = issued.ca_chain_der.split_last() else {
        bail!("the issued chain is empty");
    };
    if root_der != pinned_root {
        bail!("the issued chain does not end in the pinned root CA");
    }
    let mut child = leaf;
    let mut parsed = Vec::with_capacity(issued.ca_chain_der.len());
    for der in &issued.ca_chain_der {
        let (_, certificate) = X509Certificate::from_der(der)
            .map_err(|error| anyhow::anyhow!("a chain certificate is not valid: {error}"))?;
        parsed.push(certificate);
    }
    for parent in &parsed {
        child
            .verify_signature(Some(parent.public_key()))
            .map_err(|error| anyhow::anyhow!("the issued chain does not verify: {error}"))?;
        child = parent.clone();
    }
    if issuers.is_empty() {
        bail!("the issued chain holds no issuing CA");
    }

    Ok(Rotated {
        key_pem: pending.key.key_pem.clone(),
        certificate_der: issued.certificate_der.clone(),
        ca_chain_der: issued.ca_chain_der.clone(),
    })
}

/// The serial of a certificate without a leading zero byte, as the control plane compares it.
///
/// # Errors
///
/// Returns an error when the certificate is not valid.
pub fn serial_of(certificate_der: &[u8]) -> Result<Vec<u8>> {
    let (_, certificate) = X509Certificate::from_der(certificate_der)
        .map_err(|error| anyhow::anyhow!("the certificate is not valid: {error}"))?;
    let raw = certificate.raw_serial();
    let start = raw.iter().position(|byte| *byte != 0).unwrap_or(raw.len());
    Ok(raw[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::rotation_due;
    use time::{Duration, OffsetDateTime};

    #[test]
    fn fr_id_03_the_rotation_starts_at_two_thirds_of_the_lifetime() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = start + Duration::minutes(60);
        assert_eq!(rotation_due(start, end), start + Duration::minutes(40));
        let month = start + Duration::days(30);
        assert_eq!(rotation_due(start, month), start + Duration::days(20));
    }
}
