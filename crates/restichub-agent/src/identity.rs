//! The stored agent identity and the key pair (ADR-0026, FR-AG-04).

use std::path::Path;

use anyhow::{Context, Result, bail};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use x509_parser::prelude::FromDer as _;

use crate::{paths, pem};

pub const KEY_FILE: &str = "agent-key.pem";
pub const CERT_FILE: &str = "agent-cert.pem";
pub const CA_CHAIN_FILE: &str = "ca-chain.pem";
pub const METADATA_FILE: &str = "identity.json";
pub const SIGNING_KEY_FILE: &str = "signing-key-update.bin";

/// The identity that the agent keeps between runs (FR-AG-04).
#[derive(Debug)]
pub struct StoredIdentity {
    pub agent_id: String,
    pub tenant_id: String,
    pub cp_url: String,
    pub grpc_endpoint: Option<String>,
    pub key_pem: String,
    pub certificate_der: Vec<u8>,
    pub ca_chain_der: Vec<Vec<u8>>,
    pub signing_key_update: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    agent_id: String,
    tenant_id: String,
    cp_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    grpc_endpoint: Option<String>,
}

impl StoredIdentity {
    /// Writes the identity files into `dir` with the owner-only permissions (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when the directory or a file cannot be written.
    pub fn save(&self, dir: &Path) -> Result<()> {
        paths::ensure_config_dir(dir)?;
        paths::create_protected_file(&dir.join(KEY_FILE), self.key_pem.as_bytes())?;
        paths::create_protected_file(
            &dir.join(CERT_FILE),
            pem::encode("CERTIFICATE", &self.certificate_der).as_bytes(),
        )?;
        let chain: String = self
            .ca_chain_der
            .iter()
            .map(|der| pem::encode("CERTIFICATE", der))
            .collect();
        paths::create_protected_file(&dir.join(CA_CHAIN_FILE), chain.as_bytes())?;
        let metadata = Metadata {
            agent_id: self.agent_id.clone(),
            tenant_id: self.tenant_id.clone(),
            cp_url: self.cp_url.clone(),
            grpc_endpoint: self.grpc_endpoint.clone(),
        };
        let json = serde_json::to_vec_pretty(&metadata).context("cannot serialize the identity")?;
        paths::create_protected_file(&dir.join(METADATA_FILE), &json)?;
        paths::create_protected_file(&dir.join(SIGNING_KEY_FILE), &self.signing_key_update)?;
        Ok(())
    }

    /// Reads the identity from `dir` (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when a file is missing or malformed.
    pub fn load(dir: &Path) -> Result<Self> {
        let metadata: Metadata =
            serde_json::from_slice(&read(dir.join(METADATA_FILE))?).context("bad identity.json")?;
        let key_pem =
            String::from_utf8(read(dir.join(KEY_FILE))?).context("the key file is not text")?;
        let certificate_der = first_block(&read(dir.join(CERT_FILE))?)?;
        let ca_chain_der = pem::decode_all(
            std::str::from_utf8(&read(dir.join(CA_CHAIN_FILE))?)
                .context("the CA chain file is not text")?,
        )?;
        let signing_key_update = read(dir.join(SIGNING_KEY_FILE))?;
        Ok(Self {
            agent_id: metadata.agent_id,
            tenant_id: metadata.tenant_id,
            cp_url: metadata.cp_url,
            grpc_endpoint: metadata.grpc_endpoint,
            key_pem,
            certificate_der,
            ca_chain_der,
            signing_key_update,
        })
    }

    /// Reports whether the directory holds a stored identity.
    #[must_use]
    pub fn exists(dir: &Path) -> bool {
        dir.join(METADATA_FILE).exists()
    }

    /// The time when the stored leaf certificate stops being valid (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when the certificate is not a valid DER certificate.
    pub fn certificate_expiry(&self) -> Result<OffsetDateTime> {
        let (_, certificate) =
            x509_parser::certificate::X509Certificate::from_der(&self.certificate_der)
                .map_err(|error| anyhow::anyhow!("cannot parse the agent certificate: {error}"))?;
        Ok(certificate.validity().not_after.to_datetime())
    }
}

/// Makes an ECDSA P-256 key pair and the certificate signing request (ADR-0026).
///
/// The private key stays in memory and reaches the configuration directory only through
/// [`StoredIdentity::save`].
///
/// # Errors
///
/// Returns an error when the key or the request cannot be generated.
pub fn generate_key_and_csr() -> Result<(String, Vec<u8>)> {
    let key =
        KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).context("cannot generate the agent key")?;
    let request = CertificateParams::default()
        .serialize_request(&key)
        .context("cannot build the certificate signing request")?;
    Ok((key.serialize_pem(), request.der().to_vec()))
}

fn read(path: std::path::PathBuf) -> Result<Vec<u8>> {
    std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))
}

fn first_block(text: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(text).context("the certificate file is not text")?;
    let mut blocks = pem::decode_all(text)?;
    if blocks.is_empty() {
        bail!("the certificate file holds no PEM block");
    }
    Ok(blocks.swap_remove(0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn sample() -> StoredIdentity {
        StoredIdentity {
            agent_id: "0192f1e0-0000-7000-8000-000000000001".to_owned(),
            tenant_id: "0192f1e0-0000-7000-8000-000000000002".to_owned(),
            cp_url: "https://cp.example:8443".to_owned(),
            grpc_endpoint: Some("https://agents.example:8443".to_owned()),
            key_pem: "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n".to_owned(),
            certificate_der: vec![1, 2, 3],
            ca_chain_der: vec![vec![4, 5], vec![6, 7]],
            signing_key_update: vec![8, 9, 10],
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-identity-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_identity_round_trips() {
        let dir = temp_dir("round-trip");
        let identity = sample();
        identity.save(&dir).expect("cannot save the identity");
        assert!(StoredIdentity::exists(&dir));
        let loaded = StoredIdentity::load(&dir).expect("cannot load the identity");
        assert_eq!(loaded.agent_id, identity.agent_id);
        assert_eq!(loaded.tenant_id, identity.tenant_id);
        assert_eq!(loaded.cp_url, identity.cp_url);
        assert_eq!(loaded.grpc_endpoint, identity.grpc_endpoint);
        assert_eq!(loaded.key_pem, identity.key_pem);
        assert_eq!(loaded.certificate_der, identity.certificate_der);
        assert_eq!(loaded.ca_chain_der, identity.ca_chain_der);
        assert_eq!(loaded.signing_key_update, identity.signing_key_update);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_without_an_identity_is_reported() {
        let dir = temp_dir("absent");
        assert!(!StoredIdentity::exists(&dir));
    }

    #[test]
    fn the_generated_key_and_request_are_valid() {
        let (key_pem, csr_der) = generate_key_and_csr().expect("cannot generate the key");
        assert!(key_pem.contains("PRIVATE KEY"));
        assert!(!csr_der.is_empty());
    }
}
