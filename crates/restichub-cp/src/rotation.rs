//! Certificate rotation over the agent stream (ADR-0026, FR-ID-03, AC-ID-03).
//!
//! The agent sends a certificate signing request with a new key pair over its open mTLS stream.
//! The control plane signs a certificate for the identity of the presented certificate, never for
//! an identity in the request (FR-PR-08), keeps it as pending in the registry row and answers with
//! `CertificateIssued`. The old certificate stays valid. The agent confirms the new certificate
//! with `CertificateInstalled`, or it connects with it; the control plane then makes the pending
//! certificate the enrolled one and refuses the old serial.
//!
//! A request goes through only for an active agent whose presented certificate is the enrolled
//! one and at least half way through its lifetime. A request with the public key of the presented
//! certificate is refused, so a rotation always renews the key pair.

use restichub_db::agents as agent_repo;
use restichub_db::audit::{self, Actor, AuditAction, AuditEvent};
use restichub_db::{AgentId, AgentStatus, DbError, TenantId};
use restichub_proto::agent::v1::{CertificateIssued, CertificateRejectReason};
use time::OffsetDateTime;

use crate::pki::CaError;
use crate::server::AppState;

/// The largest accepted certificate signing request. A request for a P-256 key is far smaller.
pub const MAX_CSR_BYTES: usize = 4096;

/// The facts of the certificate that the agent presented in the TLS handshake of the stream.
#[derive(Debug, Clone)]
pub struct PresentedCertificate {
    /// The serial without a leading zero byte.
    pub serial: Vec<u8>,
    /// The raw public key.
    pub public_key: Vec<u8>,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
}

impl PresentedCertificate {
    /// Reads the facts of a certificate in DER. `None` when the bytes are not a certificate.
    pub fn from_der(der: &[u8]) -> Option<Self> {
        use x509_parser::prelude::{FromDer as _, X509Certificate};

        let (_, certificate) = X509Certificate::from_der(der).ok()?;
        let serial = strip(certificate.raw_serial()).to_vec();
        Some(Self {
            serial,
            public_key: certificate.public_key().subject_public_key.data.to_vec(),
            not_before: certificate.validity().not_before.to_datetime(),
            not_after: certificate.validity().not_after.to_datetime(),
        })
    }

    /// The time from which the control plane accepts a rotation request: half of the lifetime. The
    /// agent asks at two thirds, so the gap absorbs a clock difference between the two sides.
    pub fn rotation_opens_at(&self) -> OffsetDateTime {
        self.not_before + (self.not_after - self.not_before) / 2
    }
}

/// Signs a rotated certificate for the agent of the stream (FR-ID-03).
///
/// The facts of the new certificate come with it, so the stream accepts the next rotation after
/// the confirmation.
///
/// # Errors
///
/// Returns the reason of the refusal. The old certificate stays valid in every case.
pub async fn issue(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    presented: &PresentedCertificate,
    csr_der: &[u8],
    now: OffsetDateTime,
) -> Result<(CertificateIssued, PresentedCertificate), CertificateRejectReason> {
    if csr_der.is_empty() || csr_der.len() > MAX_CSR_BYTES {
        return Err(CertificateRejectReason::InvalidRequest);
    }
    if now < presented.rotation_opens_at() {
        return Err(CertificateRejectReason::TooEarly);
    }
    let new_key = crate::pki::csr_public_key(csr_der).map_err(refusal)?;
    if new_key == presented.public_key {
        return Err(CertificateRejectReason::SameKey);
    }
    let store = state.ca().ok_or(CertificateRejectReason::Unavailable)?;

    let unavailable = |error: DbError| {
        tracing::warn!(%agent, cause = ?error, "the certificate rotation failed");
        CertificateRejectReason::Unavailable
    };
    let mut tx = state.db().begin_tenant(tenant).await.map_err(unavailable)?;
    // The lock serializes the rotation with a revoke and with the confirmation of an earlier one.
    let row = agent_repo::get_for_update(&mut tx, agent)
        .await
        .map_err(unavailable)?
        .ok_or(CertificateRejectReason::Revoked)?;
    if row.status != AgentStatus::Active {
        return Err(CertificateRejectReason::Revoked);
    }
    if strip(&row.certificate_serial) != presented.serial.as_slice() {
        return Err(CertificateRejectReason::InvalidRequest);
    }

    let issued = store
        .issue_agent_certificate_at(
            tenant.as_uuid(),
            agent.as_uuid(),
            csr_der,
            now,
            state.agent_cert_lifetime(),
        )
        .map_err(refusal)?;
    agent_repo::set_pending_certificate(&mut tx, agent, &issued.serial, issued.not_after)
        .await
        .map_err(unavailable)?;

    let mut event = AuditEvent::new(AuditAction::AgentCertificateRotate, "agent");
    event.actor = Actor::Agent(agent.as_uuid());
    event.object_id = Some(agent.as_uuid());
    event.diff = Some(audit::diff(
        Some(&serde_json::json!({
            "certificate_serial": hex(&row.certificate_serial),
            "certificate_expires_at": row.certificate_expires_at.to_string(),
        })),
        Some(&serde_json::json!({
            "certificate_serial": hex(&issued.serial),
            "certificate_expires_at": issued.not_after.to_string(),
        })),
        &[],
    ));
    audit::insert(&mut tx, &event).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)?;

    let next = PresentedCertificate::from_der(&issued.cert_der)
        .ok_or(CertificateRejectReason::Unavailable)?;
    Ok((
        CertificateIssued {
            certificate_der: issued.cert_der,
            ca_chain_der: issued.chain_der,
        },
        next,
    ))
}

/// Makes the pending certificate the enrolled one (ADR-0026).
///
/// `true` when the serial named the pending certificate of an active agent. The old serial is
/// refused from then on.
///
/// # Errors
///
/// Returns an error when the registry is not reachable.
pub async fn confirm(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    serial: &[u8],
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let mut tx = state.db().begin_tenant(tenant).await?;
    agent_repo::get_for_update(&mut tx, agent).await?;
    let promoted = agent_repo::promote_pending_certificate(&mut tx, agent, serial, now).await?;
    tx.commit().await?;
    Ok(promoted)
}

/// Maps a certificate authority error to the reason that the agent sees.
fn refusal(error: CaError) -> CertificateRejectReason {
    match error {
        CaError::BadCsr(_)
        | CaError::CsrCaFlag
        | CaError::CsrExtraSan
        | CaError::CsrKeyAlgorithm
        | CaError::BadSan { .. } => CertificateRejectReason::InvalidRequest,
        other => {
            tracing::warn!(cause = %other, "the certificate authority cannot sign a rotation");
            CertificateRejectReason::Unavailable
        }
    }
}

fn strip(serial: &[u8]) -> &[u8] {
    let start = serial
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(serial.len());
    &serial[start..]
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::PresentedCertificate;
    use time::{Duration, OffsetDateTime};

    #[test]
    fn fr_id_03_the_rotation_opens_at_half_of_the_lifetime() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let presented = PresentedCertificate {
            serial: vec![1],
            public_key: vec![2],
            not_before: start,
            not_after: start + Duration::minutes(60),
        };
        assert_eq!(presented.rotation_opens_at(), start + Duration::minutes(30));
    }
}
