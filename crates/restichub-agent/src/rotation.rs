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
use x509_parser::prelude::{FromDer as _, X509Certificate};

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
/// The certificate gets the checks of an enrollment: the identity of this agent, the new public
/// key, the validity windows and the CA constraints of the chain. The chain must end in the root
/// that the agent pinned at enrollment; the chain never replaces the pinned root.
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
    let pinned_root = identity
        .ca_chain_der
        .last()
        .context("the identity holds no pinned root")?;
    if issued.ca_chain_der.last() != Some(pinned_root) {
        bail!("the issued chain does not end in the pinned root CA");
    }
    crate::verify::check_certificate(
        &pending.key.public_key,
        &identity.agent_id,
        &identity.tenant_id,
        &issued.certificate_der,
        &issued.ca_chain_der,
        now,
    )?;
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
    #![allow(clippy::expect_used)]

    use super::{CertificateIssued, accept, begin, rotation_due};
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair,
        PKCS_ECDSA_P256_SHA256, SanType,
    };
    use time::{Duration, OffsetDateTime};

    fn ca(
        path_len: Option<u8>,
        issuer: Option<&CertifiedIssuer<'static, KeyPair>>,
    ) -> CertifiedIssuer<'static, KeyPair> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("CA key");
        let mut params = CertificateParams::new(Vec::new()).expect("CA parameters");
        params.is_ca = IsCa::Ca(path_len.map_or(
            BasicConstraints::Unconstrained,
            BasicConstraints::Constrained,
        ));
        match issuer {
            Some(issuer) => CertifiedIssuer::signed_by(params, key, issuer).expect("issuing CA"),
            None => CertifiedIssuer::self_signed(params, key).expect("root CA"),
        }
    }

    #[test]
    fn fr_id_03_a_rotation_applies_the_chain_checks_of_an_enrollment() {
        let root = ca(None, None);
        let issuing = ca(Some(0), Some(&root));
        let mut identity = crate::identity::tests::sample();
        identity.ca_chain_der = vec![issuing.der().to_vec(), root.der().to_vec()];
        let san = crate::verify::san_uri(&identity.tenant_id, &identity.agent_id);
        let (pending, _) = begin().expect("a pending rotation");
        let key = KeyPair::from_pem(&pending.key.key_pem).expect("the pending key");
        let issue = |is_ca: IsCa, chain: Vec<Vec<u8>>| {
            let mut params = CertificateParams::new(Vec::new()).expect("leaf parameters");
            params
                .subject_alt_names
                .push(SanType::URI(san.clone().try_into().expect("SAN URI")));
            params.is_ca = is_ca;
            CertificateIssued {
                certificate_der: params
                    .signed_by(&key, &issuing)
                    .expect("leaf")
                    .der()
                    .to_vec(),
                ca_chain_der: chain,
            }
        };
        let now = OffsetDateTime::now_utc();

        accept(
            &identity,
            &pending,
            &issue(IsCa::ExplicitNoCa, identity.ca_chain_der.clone()),
            now,
        )
        .expect("an end entity passes");
        let error = accept(
            &identity,
            &pending,
            &issue(
                IsCa::Ca(BasicConstraints::Unconstrained),
                identity.ca_chain_der.clone(),
            ),
            now,
        )
        .expect_err("a leaf that is a CA");
        assert!(
            format!("{error:#}").contains("is a certificate authority"),
            "{error:#}"
        );
        let other_root = ca(None, None);
        let error = accept(
            &identity,
            &pending,
            &issue(
                IsCa::ExplicitNoCa,
                vec![issuing.der().to_vec(), other_root.der().to_vec()],
            ),
            now,
        )
        .expect_err("another root");
        assert!(format!("{error:#}").contains("pinned root"), "{error:#}");
    }

    #[test]
    fn fr_id_03_the_rotation_starts_at_two_thirds_of_the_lifetime() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let end = start + Duration::minutes(60);
        assert_eq!(rotation_due(start, end), start + Duration::minutes(40));
        let month = start + Duration::days(30);
        assert_eq!(rotation_due(start, month), start + Duration::days(20));
    }
}
