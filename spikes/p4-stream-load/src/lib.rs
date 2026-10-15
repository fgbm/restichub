//! Spike P4: shared code for the control plane stand-in and the agent
//! emulator.
//!
//! The module builds the test certificate authority, the job signing key and
//! the sample job. It uses the real `restichub-proto` signing and validation
//! code. See [`design/spikes/p4-stream-load.md`](../../../design/spikes/p4-stream-load.md).

use std::path::Path;

use ed25519_dalek::{SigningKey, VerifyingKey};
use prost_types::Timestamp;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use restichub_proto::agent::v1::{
    BackupStep, Job, LocalCredentials, RepositoryAccess, Step,
    repository_access::BackendCredentials, step,
};
use restichub_proto::signing::KeyRing;

/// Identifier of the tenant of every emulated agent.
pub const TENANT_ID: &str = "tenant-1";

/// Identifier of the job signing key.
pub const KEY_ID: &str = "job-key-1";

/// Identifier of the one repository of a sample job.
pub const REPOSITORY_ID: &str = "repo-1";

/// Domain name of the control plane certificate.
pub const SERVER_NAME: &str = "localhost";

/// Grace period after a control plane start, in seconds (AC-ID-06 uses 300).
pub const GRACE_SECONDS: u64 = 300;

/// Heartbeat interval of the control plane, in seconds (FR-ID-07).
pub const PING_INTERVAL_SECONDS: u64 = 30;

/// The job signing key. The seed is fixed so that the server and the emulator
/// use the same key. A spike does not rotate keys.
#[must_use]
pub fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// The public job signing key that the emulated agents trust.
#[must_use]
pub fn verifying_key() -> VerifyingKey {
    signing_key().verifying_key()
}

/// A key ring that holds the one job signing key.
#[must_use]
pub fn key_ring() -> KeyRing {
    let mut ring = KeyRing::new();
    let _ = ring.insert(KEY_ID, verifying_key());
    ring
}

/// Returns the current time as a Protocol Buffers timestamp.
#[must_use]
pub fn now_timestamp() -> Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}

/// Builds a small backup job for `agent_id`. The job passes `validate_job`.
#[must_use]
pub fn sample_job(agent_id: &str, job_id: &str) -> Job {
    let issued = now_timestamp();
    let expires_at = Timestamp {
        seconds: issued.seconds + 300,
        nanos: issued.nanos,
    };
    Job {
        job_id: job_id.to_owned(),
        tenant_id: TENANT_ID.to_owned(),
        agent_id: agent_id.to_owned(),
        issued_at: Some(issued),
        expires_at: Some(expires_at),
        timeout_seconds: 60,
        stall_timeout_seconds: 0,
        repositories: vec![RepositoryAccess {
            repository_id: REPOSITORY_ID.to_owned(),
            location: "/tmp/restichub-p4-repo".to_owned(),
            password: "spike-password".to_owned(),
            backend_credentials: Some(BackendCredentials::Local(LocalCredentials {})),
        }],
        steps: vec![Step {
            step_id: "step-1".to_owned(),
            kind: Some(step::Kind::Backup(BackupStep {
                repository_id: REPOSITORY_ID.to_owned(),
                paths: vec!["/srv".to_owned()],
                ..BackupStep::default()
            })),
            ..Step::default()
        }],
    }
}

/// Returns the agent identifier of an emulated agent.
#[must_use]
pub fn agent_id(index: usize) -> String {
    format!("agent-{index:05}")
}

/// The PEM material of one emulated agent.
pub struct ClientCert {
    /// Agent identifier, from the certificate common name.
    pub agent_id: String,
    /// Client certificate in PEM.
    pub cert_pem: String,
    /// Client private key in PEM.
    pub key_pem: String,
}

/// The PEM material of the certificate authority and the server.
pub struct CertSet {
    /// Certificate authority certificate in PEM.
    pub ca_pem: String,
    /// Server certificate in PEM.
    pub server_cert_pem: String,
    /// Server private key in PEM.
    pub server_key_pem: String,
    /// One entry for each emulated agent.
    pub clients: Vec<ClientCert>,
}

/// Builds a certificate authority, a server certificate and `count` client
/// certificates, one for each agent identifier.
///
/// # Errors
///
/// Returns an error when `rcgen` cannot generate a certificate.
pub fn generate_certs(count: usize) -> anyhow::Result<CertSet> {
    let mut ca_params = CertificateParams::new(Vec::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "restichub-p4-ca");
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = KeyPair::generate()?;
    let ca_issuer = CertifiedIssuer::self_signed(ca_params, ca_key)?;

    let mut server_params =
        CertificateParams::new(vec![SERVER_NAME.to_owned(), "127.0.0.1".to_owned()])?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, "restichub-p4-server");
    server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_key = KeyPair::generate()?;
    let server_cert = server_params.signed_by(&server_key, &ca_issuer)?;

    let mut clients = Vec::with_capacity(count);
    for index in 0..count {
        let id = agent_id(index);
        let mut params = CertificateParams::new(vec![id.clone()])?;
        params
            .distinguished_name
            .push(DnType::CommonName, id.clone());
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let key = KeyPair::generate()?;
        let cert = params.signed_by(&key, &ca_issuer)?;
        clients.push(ClientCert {
            agent_id: id,
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
        });
    }

    Ok(CertSet {
        ca_pem: ca_issuer.pem(),
        server_cert_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
        clients,
    })
}

/// Writes the certificate set to `dir`. The function writes the server files
/// at the top level and one file pair for each agent under `dir/clients`.
///
/// # Errors
///
/// Returns an error when the file system rejects a write.
pub fn write_certs(dir: &Path, set: &CertSet) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir.join("clients"))?;
    std::fs::write(dir.join("ca.pem"), &set.ca_pem)?;
    std::fs::write(dir.join("server.pem"), &set.server_cert_pem)?;
    std::fs::write(dir.join("server.key"), &set.server_key_pem)?;
    let mut agents = Vec::with_capacity(set.clients.len());
    for client in &set.clients {
        std::fs::write(
            dir.join("clients").join(format!("{}.pem", client.agent_id)),
            &client.cert_pem,
        )?;
        std::fs::write(
            dir.join("clients").join(format!("{}.key", client.agent_id)),
            &client.key_pem,
        )?;
        agents.push(client.agent_id.clone());
    }
    std::fs::write(dir.join("agents.txt"), agents.join("\n"))?;
    Ok(())
}

/// Reads the client certificate pair of `agent_id` from `dir`.
///
/// # Errors
///
/// Returns an error when a file cannot be read.
pub fn read_client(dir: &Path, agent_id: &str) -> anyhow::Result<(String, String)> {
    let cert = std::fs::read_to_string(dir.join("clients").join(format!("{agent_id}.pem")))?;
    let key = std::fs::read_to_string(dir.join("clients").join(format!("{agent_id}.key")))?;
    Ok((cert, key))
}

/// Extracts the common name of a DER certificate.
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
