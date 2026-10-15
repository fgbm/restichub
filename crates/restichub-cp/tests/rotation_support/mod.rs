//! Helpers of the certificate rotation tests: the agent side material and the registry row.

#![allow(clippy::expect_used, dead_code)]

use rcgen::string::Ia5String;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256, SanType};
use restichub_agent::identity::{Rotated, StoredIdentity, StoredSigningKeys};
use restichub_db::AgentId;
use restichub_emulator::enroll::EnrolledIdentity;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::queue_support::Env;

pub fn agent_of(identity: &EnrolledIdentity) -> AgentId {
    AgentId::from_uuid(Uuid::parse_str(&identity.agent_id).expect("agent id"))
}

/// The identity as the real agent keeps it on disk after the enrollment.
pub fn stored_identity(identity: &EnrolledIdentity) -> StoredIdentity {
    StoredIdentity {
        agent_id: identity.agent_id.clone(),
        tenant_id: identity.tenant_id.clone(),
        cp_url: "https://cp.example".to_owned(),
        grpc_endpoint: identity.grpc_endpoint.clone(),
        key_pem: identity.key_pem.clone(),
        certificate_der: identity.certificate_der.clone(),
        ca_chain_der: identity.ca_chain_der.clone(),
        signing_keys: StoredSigningKeys {
            keys: Vec::new(),
            signature: String::new(),
        },
    }
}

/// The identity after the agent installed a rotated certificate.
pub fn rotated_identity(identity: &EnrolledIdentity, rotated: &Rotated) -> EnrolledIdentity {
    EnrolledIdentity {
        certificate_der: rotated.certificate_der.clone(),
        ca_chain_der: rotated.ca_chain_der.clone(),
        key_pem: rotated.key_pem.clone(),
        ..identity.clone()
    }
}

/// A request for the key `key_pem`, as an agent that reuses its key would send it.
pub fn csr_for_key(key_pem: &str) -> Vec<u8> {
    let key = KeyPair::from_pem(key_pem).expect("the key parses");
    CertificateParams::default()
        .serialize_request(&key)
        .expect("the request builds")
        .der()
        .to_vec()
}

/// A request with a new key and the given identity URI as a subject alternative name.
pub fn csr_with_uri(uri: &str) -> Vec<u8> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("a key");
    let mut params = CertificateParams::default();
    params.subject_alt_names = vec![SanType::URI(
        Ia5String::try_from(uri.to_owned()).expect("a URI"),
    )];
    params
        .serialize_request(&key)
        .expect("the request builds")
        .der()
        .to_vec()
}

/// The certificate columns of the registry row.
#[derive(Debug)]
pub struct Row {
    pub serial: String,
    pub pending_serial: Option<String>,
    pub expires_at: OffsetDateTime,
    pub last_rotation: Option<OffsetDateTime>,
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

pub async fn row(env: &Env, agent: AgentId) -> Row {
    let mut tx = env.scoped().await;
    let (serial, pending_serial, expires_at, last_rotation): (
        String,
        Option<String>,
        OffsetDateTime,
        Option<OffsetDateTime>,
    ) = sqlx::query_as(
        "SELECT encode(certificate_serial, 'hex'), encode(pending_certificate_serial, 'hex'), \
                certificate_expires_at, last_certificate_rotation_at \
         FROM agents WHERE id = $1",
    )
    .bind(agent)
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the agent row");
    Row {
        serial,
        pending_serial,
        expires_at,
        last_rotation,
    }
}

/// The number of audit records of the rotation action in the tenant of the environment.
pub async fn rotation_audit_count(env: &Env) -> i64 {
    let mut tx = env.scoped().await;
    sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'agent_certificate_rotate'")
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count the audit records")
}
