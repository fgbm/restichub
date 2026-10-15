//! Certificate rotation of an emulated agent (FR-ID-03, ADR-0026).
//!
//! The emulated agent asks for a new certificate over its stream at two thirds of the lifetime,
//! as the real agent does. It keeps the renewed material in memory and uses it for the next
//! connection. An emulated agent with a long-lived test certificate never rotates.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use restichub_proto::agent::v1::CertificateIssued;
use x509_parser::prelude::{FromDer as _, X509Certificate};

use crate::identity::AgentIdentity;

/// The longest wait for a rotation that the emulator arms. A longer wait means a test certificate
/// with a validity of years, which never rotates.
const MAX_WAIT: Duration = Duration::from_secs(100 * 24 * 60 * 60);

/// The rotation state that all connections of one emulated agent share.
pub struct Renewal {
    renewed: Mutex<Option<AgentIdentity>>,
    completed: AtomicU32,
    /// The wait before a refused request is repeated.
    pub retry: Duration,
    /// Unix seconds. A test replaces the clock.
    pub now: fn() -> i64,
}

impl Default for Renewal {
    fn default() -> Self {
        Self {
            renewed: Mutex::new(None),
            completed: AtomicU32::new(0),
            retry: Duration::from_secs(300),
            now: system_now,
        }
    }
}

fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

impl Renewal {
    /// The identity that the next connection presents: the renewed one, or the configured one.
    pub fn identity(&self, configured: &AgentIdentity) -> AgentIdentity {
        crate::lock(&self.renewed)
            .clone()
            .unwrap_or_else(|| configured.clone())
    }

    /// The number of certificates that the emulated agent stored.
    pub fn completed(&self) -> u32 {
        self.completed.load(Ordering::Relaxed)
    }

    /// The wait until two thirds of the validity of the certificate in `cert_pem` have passed.
    /// `None` when the certificate is unreadable or valid for years.
    pub fn wait_until_due(&self, cert_pem: &str) -> Option<Duration> {
        let der = first_certificate(cert_pem)?;
        let (_, certificate) = X509Certificate::from_der(&der).ok()?;
        let not_before = certificate.validity().not_before.timestamp();
        let not_after = certificate.validity().not_after.timestamp();
        let due = not_before + (not_after - not_before) * 2 / 3;
        let wait = Duration::from_secs(u64::try_from(due - (self.now)()).unwrap_or(0));
        (wait <= MAX_WAIT).then_some(wait)
    }

    /// Stores the issued certificate and returns its serial for the confirmation.
    pub fn accept(
        &self,
        pending: &PendingKey,
        issued: &CertificateIssued,
        agent_id: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let (_, certificate) = X509Certificate::from_der(&issued.certificate_der)
            .map_err(|error| anyhow::anyhow!("the issued certificate is not valid: {error}"))?;
        anyhow::ensure!(
            certificate.public_key().subject_public_key.data.as_ref() == pending.public_key,
            "the issued certificate does not hold the requested key"
        );
        let mut cert_pem = pem("CERTIFICATE", &issued.certificate_der);
        for der in &issued.ca_chain_der {
            cert_pem.push_str(&pem("CERTIFICATE", der));
        }
        *crate::lock(&self.renewed) = Some(AgentIdentity {
            agent_id: agent_id.to_owned(),
            cert_pem,
            key_pem: pending.key_pem.clone(),
        });
        self.completed.fetch_add(1, Ordering::Relaxed);
        let raw = certificate.raw_serial();
        let start = raw.iter().position(|byte| *byte != 0).unwrap_or(raw.len());
        Ok(raw[start..].to_vec())
    }
}

/// A new key pair that waits for its certificate.
pub struct PendingKey {
    key_pem: String,
    public_key: Vec<u8>,
}

/// Makes a new key pair and the DER certificate signing request for it.
///
/// # Errors
///
/// Returns an error when the key or the request cannot be generated.
pub fn new_request() -> anyhow::Result<(PendingKey, Vec<u8>)> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).context("cannot generate a key")?;
    let request = CertificateParams::default()
        .serialize_request(&key)
        .context("cannot build the request")?;
    Ok((
        PendingKey {
            key_pem: key.serialize_pem(),
            public_key: key.public_key_raw().to_vec(),
        },
        request.der().to_vec(),
    ))
}

fn pem(label: &str, der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let mut text = format!("-----BEGIN {label}-----\n");
    for chunk in body.as_bytes().chunks(64) {
        text.push_str(&String::from_utf8_lossy(chunk));
        text.push('\n');
    }
    text.push_str(&format!("-----END {label}-----\n"));
    text
}

fn first_certificate(text: &str) -> Option<Vec<u8>> {
    let start = text.find("-----BEGIN CERTIFICATE-----")? + "-----BEGIN CERTIFICATE-----".len();
    let end = text[start..].find("-----END CERTIFICATE-----")? + start;
    let body: String = text[start..end].split_whitespace().collect();
    STANDARD.decode(body).ok()
}

#[cfg(test)]
mod tests {
    use super::{first_certificate, pem};

    #[test]
    fn fr_id_03_a_certificate_survives_the_pem_round_trip() {
        let der = vec![1_u8, 2, 3, 4, 5];
        let text = pem("CERTIFICATE", &der);
        assert_eq!(first_certificate(&text), Some(der));
    }
}
