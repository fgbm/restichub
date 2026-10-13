//! Test identity material for the emulated agents.
//!
//! The real agent gets its certificate from the enrollment endpoint (ADR-0026,
//! epic #1). Until that lands the emulator makes its own certificate authority
//! and one client certificate for each agent with `rcgen`. The material stays
//! in memory unless `--write-certs` asks for files. The real certificate
//! carries the agent identity in the SAN URI (ADR-0026); the test certificate
//! uses the common name, which the test control plane reads. Epic #1 replaces
//! this module with the identity from enrollment.

use std::path::{Path, PathBuf};

use ed25519_dalek::{SigningKey, VerifyingKey};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use restichub_proto::signing::KeyRing;

/// Tenant identifier of every emulated agent.
pub const TENANT_ID: &str = "tenant-1";

/// Identifier of the test job signing key.
pub const KEY_ID: &str = "job-key-1";

/// Domain of the test server certificate.
pub const SERVER_NAME: &str = "localhost";

/// Private Ed25519 key that makes each job signature. The seed is fixed, so
/// the test control plane and the emulator agree without a key exchange. Epic
/// #1 replaces it with the key set from the control plane (ADR-0026).
#[must_use]
pub fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// The public test job signing key that the emulated agents trust.
#[must_use]
pub fn verifying_key() -> VerifyingKey {
    signing_key().verifying_key()
}

/// Builds the trust set for the one test job signing key.
#[must_use]
pub fn key_ring() -> KeyRing {
    let mut ring = KeyRing::new();
    // The fixed test key cannot be revoked or conflict.
    let _ = ring.insert(KEY_ID, verifying_key());
    ring
}

/// Agent identifier of an emulated agent.
#[must_use]
pub fn agent_id(index: usize) -> String {
    format!("agent-{index:05}")
}

/// Certificate and key material of one emulated agent.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub cert_pem: String,
    pub key_pem: String,
}

/// The test certificate authority, the server certificate and one client
/// identity for each emulated agent.
#[derive(Debug, Clone)]
pub struct TestPki {
    ca_pem: String,
    server_cert_pem: String,
    server_key_pem: String,
    agents: Vec<AgentIdentity>,
}

impl TestPki {
    /// Makes a certificate authority, a server certificate and `agent_count`
    /// client certificates, one for each agent identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when `rcgen` cannot generate a certificate.
    pub fn generate(agent_count: usize) -> anyhow::Result<Self> {
        let mut ca_params = CertificateParams::new(Vec::new())?;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "restichub-emulator-ca");
        ca_params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let ca_issuer = CertifiedIssuer::self_signed(ca_params, KeyPair::generate()?)?;

        let mut server_params =
            CertificateParams::new(vec![SERVER_NAME.to_owned(), "127.0.0.1".to_owned()])?;
        server_params
            .distinguished_name
            .push(DnType::CommonName, "restichub-emulator-server");
        server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_key = KeyPair::generate()?;
        let server_cert = server_params.signed_by(&server_key, &ca_issuer)?;

        let mut agents = Vec::with_capacity(agent_count);
        for index in 0..agent_count {
            let id = agent_id(index);
            let mut params = CertificateParams::new(vec![id.clone()])?;
            params
                .distinguished_name
                .push(DnType::CommonName, id.clone());
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
            let key = KeyPair::generate()?;
            let cert = params.signed_by(&key, &ca_issuer)?;
            agents.push(AgentIdentity {
                agent_id: id,
                cert_pem: cert.pem(),
                key_pem: key.serialize_pem(),
            });
        }

        Ok(Self {
            ca_pem: ca_issuer.pem(),
            server_cert_pem: server_cert.pem(),
            server_key_pem: server_key.serialize_pem(),
            agents,
        })
    }

    #[must_use]
    pub fn ca_pem(&self) -> &str {
        &self.ca_pem
    }

    #[must_use]
    pub fn server_cert_pem(&self) -> &str {
        &self.server_cert_pem
    }

    #[must_use]
    pub fn server_key_pem(&self) -> &str {
        &self.server_key_pem
    }

    #[must_use]
    pub fn agents(&self) -> &[AgentIdentity] {
        &self.agents
    }

    /// Reads the authority, the server files and the client identities from
    /// `dir`.
    ///
    /// # Errors
    ///
    /// Fails when a file cannot be read.
    pub fn read(dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            ca_pem: read_ca(dir)?,
            server_cert_pem: std::fs::read_to_string(dir.join("server.pem"))?,
            server_key_pem: std::fs::read_to_string(dir.join("server.key"))?,
            agents: read_client_identities(dir)?,
        })
    }

    /// Writes the authority, the server files and one file pair for each agent
    /// under `dir/clients`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file system rejects a write.
    pub fn write(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir.join("clients"))?;
        std::fs::write(dir.join("ca.pem"), &self.ca_pem)?;
        std::fs::write(dir.join("server.pem"), &self.server_cert_pem)?;
        std::fs::write(dir.join("server.key"), &self.server_key_pem)?;
        let mut ids = Vec::with_capacity(self.agents.len());
        for agent in &self.agents {
            std::fs::write(
                dir.join("clients").join(format!("{}.pem", agent.agent_id)),
                &agent.cert_pem,
            )?;
            std::fs::write(
                dir.join("clients").join(format!("{}.key", agent.agent_id)),
                &agent.key_pem,
            )?;
            ids.push(agent.agent_id.clone());
        }
        std::fs::write(dir.join("agents.txt"), ids.join("\n"))?;
        Ok(())
    }
}

/// Reads the certificate authority from `dir/ca.pem`.
///
/// # Errors
///
/// Fails when the file cannot be read.
pub fn read_ca(dir: &Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(dir.join("ca.pem"))?)
}

/// Reads the client identity of each agent from `dir`. The function uses
/// `agents.txt` for the order.
///
/// # Errors
///
/// Fails when a file cannot be read.
pub fn read_client_identities(dir: &Path) -> anyhow::Result<Vec<AgentIdentity>> {
    let list = std::fs::read_to_string(dir.join("agents.txt"))?;
    let mut identities = Vec::new();
    for id in list.lines().filter(|line| !line.is_empty()) {
        let cert_pem = std::fs::read_to_string(clients_path(dir, id).with_extension("pem"))?;
        let key_pem = std::fs::read_to_string(clients_path(dir, id).with_extension("key"))?;
        identities.push(AgentIdentity {
            agent_id: id.to_owned(),
            cert_pem,
            key_pem,
        });
    }
    Ok(identities)
}

fn clients_path(dir: &Path, agent_id: &str) -> PathBuf {
    dir.join("clients").join(agent_id)
}

/// Reads the subject CN of a DER certificate.
///
/// # Errors
///
/// Returns an error when the bytes are not a valid certificate or the common
/// name is absent.
pub fn common_name(der: &[u8]) -> anyhow::Result<String> {
    let (_, certificate) = x509_parser::parse_x509_certificate(der)
        .map_err(|error| anyhow::anyhow!("cannot parse certificate: {error}"))?;
    let name = certificate
        .subject()
        .iter_common_name()
        .next()
        .and_then(|attribute| attribute.as_str().ok())
        .ok_or_else(|| anyhow::anyhow!("certificate has no common name"))?;
    Ok(name.to_owned())
}
