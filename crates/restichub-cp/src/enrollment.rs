//! The enrollment exchange (ADR-0026, FR-ID-01, FR-ID-02, FR-ID-05, FR-TN-03, AC-ID-01).
//!
//! The endpoint is the only control plane route without a session, a cookie or a client
//! certificate (FR-ID-05). The agent sends a one-time token, a PKCS#10 certificate signing request
//! with an ECDSA P-256 key and its host facts. The control plane consumes the token, issues the
//! agent certificate from the issuing CA, creates the registry row and returns the certificate, the
//! CA chain, the signed job signing key set and the gRPC endpoint.
//!
//! The identity comes from the token and the certificate, never from a request field (FR-PR-08).
//! The tenant is the tenant of the consumed token (FR-TN-03). The unknown, used, expired and
//! revoked cases give one generic refusal, so the client learns no difference; only the audit
//! record names the reason (AC-IN-02).
//!
//! The consume of the token and the create of the agent row run in one transaction, so a consumed
//! token never stays without an agent. The refusal record commits in the transaction of the
//! refused token's tenant.
//!
//! The request is checked and the signed key set is built before the token is consumed, so a bad
//! request or a missing key set never spends a token and never reveals the token state.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use prost::Message as _;
use restichub_db::agents::{self as agent_repo, NewAgent};
use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::enrollment_tokens::{self, EnrollmentConsume, EnrollmentRefusal};
use restichub_db::{AgentId, EnrollmentTokenId, TenantTx};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::client_ip::ClientIp;
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::pki::CaError;
use crate::server::AppState;
use crate::validation::{self, Reason};

/// The token prefix that the response of a refused request does not reveal.
const TOKEN_PREFIX_LEN: usize = 8;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollRequest {
    /// The one-time secret that the Web UI shows, `re_...`.
    pub token: String,
    /// The PKCS#10 certificate signing request in DER, base64 (standard alphabet).
    pub csr: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
    pub restic_version: String,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub agent_id: String,
    pub tenant_id: String,
    /// The issued leaf in DER, base64 (standard alphabet).
    pub certificate: String,
    /// The issuing CA and the root CA, in DER, base64, in that order.
    pub ca_chain: Vec<String>,
    /// In RFC 3339.
    pub certificate_expires_at: String,
    /// The signed job signing key set, the serialized `SigningKeyUpdate` message in base64. The
    /// agent verifies the update before it uses the next key (FR-PR-04, FR-PR-11).
    pub signing_key_update: String,
    /// The public job signing key set: the current key and the next key (ADR-0026). The agent
    /// verifies a job that the current key signed with this set at once.
    pub signing_keys: Vec<EnrollmentKeyBody>,
    /// Ed25519 signature over the key set, base64 (standard alphabet). The current key signs it
    /// (ADR-0026).
    pub signing_keys_signature: String,
    /// The gRPC endpoint of the agent listener, when the installation sets it.
    pub grpc_endpoint: Option<String>,
}

/// One public job signing key of the enrollment response (ADR-0026).
#[derive(Debug, Serialize, ToSchema)]
pub struct EnrollmentKeyBody {
    /// `current` or `next`.
    pub role: String,
    pub key_id: String,
    /// Base64 of the 32 Ed25519 bytes (standard alphabet).
    pub public_key: String,
}

/// Enrolls an agent with a one-time token and a certificate signing request (FR-ID-01, FR-ID-02).
///
/// The route lives on the agent listener, not on the public web API (ADR-0026).
pub async fn enroll(
    State(state): State<AppState>,
    ClientIp(source_ip): ClientIp,
    ApiJson(body): ApiJson<EnrollRequest>,
) -> Result<(StatusCode, Json<EnrollResponse>), ApiError> {
    limit(&state, source_ip, &body.token)?;
    let csr_der = decode_csr(&body.csr)?;
    let hostname = validation::validate_display_name("hostname", &body.hostname)?;
    let os = validation::validate_token_name("os", &body.os)?;
    let arch = validation::validate_token_name("arch", &body.arch)?;
    let agent_version = validation::validate_token_name("agent_version", &body.agent_version)?;
    let restic_version = validation::validate_token_name("restic_version", &body.restic_version)?;

    // The request checks run before the token lookup: a bad request never spends a token and never
    // reveals the token state (AC-IN-02).
    crate::pki::validate_enrollment_csr(&csr_der).map_err(map_ca_error)?;
    let store = state
        .ca()
        .ok_or_else(|| ApiError::internal("the certificate authority is not loaded"))?;
    let job_signing = state
        .job_signing()
        .ok_or_else(|| ApiError::internal("the job signing key set is not loaded"))?;

    let now = state.auth().clock.now();
    // Build the signed key set before the consume, so a missing key set fails with the token
    // untouched and no agent row.
    let update = job_signing.key_update(state.db(), now).await?;
    let mut key_update_bytes = Vec::new();
    update
        .encode(&mut key_update_bytes)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    // The full public key set, so the agent verifies a job of the current key at once (ADR-0026).
    let (signing_keys, signing_keys_signature) = job_signing.enrollment_keys(state.db()).await?;
    let signing_keys = signing_keys
        .into_iter()
        .map(|key| EnrollmentKeyBody {
            role: key.role,
            key_id: key.key_id,
            public_key: STANDARD.encode(&key.public_key),
        })
        .collect();

    let token_hash = crate::token::hash_token(&body.token);
    let mut tx = state.db().begin_enrollment().await?;
    let consume = enrollment_tokens::consume_for_enrollment(&mut tx, &token_hash).await?;

    match consume {
        EnrollmentConsume::Unknown => {
            tx.commit().await?;
            // The token value never reaches a log or the client (T9, T21).
            tracing::info!(%source_ip, "enrollment refused: unknown token");
            Err(refused())
        }
        EnrollmentConsume::Refused {
            tenant_id,
            token_id,
            reason,
        } => {
            let mut tx = tx.into_tenant_tx()?;
            write_refusal(&mut tx, token_id, reason).await?;
            tx.commit().await?;
            tracing::info!(%source_ip, %tenant_id, reason = reason.as_str(), "enrollment refused");
            Err(refused())
        }
        EnrollmentConsume::Consumed {
            tenant_id,
            token_id,
            labels,
        } => {
            let agent_id = AgentId::new();
            let issued = store
                .issue_agent_certificate(tenant_id.as_uuid(), agent_id.as_uuid(), &csr_der)
                .map_err(map_ca_error)?;

            let mut tx = tx.into_tenant_tx()?;
            agent_repo::create(
                &mut tx,
                &NewAgent {
                    id: agent_id,
                    hostname: &hostname,
                    os: &os,
                    arch: &arch,
                    agent_version: &agent_version,
                    restic_version: &restic_version,
                    labels: &labels,
                    certificate_serial: &issued.serial,
                    certificate_expires_at: issued.not_after,
                    last_seen_at: Some(now),
                },
            )
            .await?;
            write_use(&mut tx, token_id, agent_id, source_ip).await?;
            write_enroll(
                &mut tx,
                agent_id,
                source_ip,
                &EnrollFacts {
                    hostname: &hostname,
                    os: &os,
                    arch: &arch,
                    agent_version: &agent_version,
                    restic_version: &restic_version,
                    labels: &labels,
                    serial: &issued.serial,
                    not_after: issued.not_after,
                },
            )
            .await?;
            tx.commit().await?;

            tracing::info!(%source_ip, %tenant_id, %agent_id, "agent enrolled");
            Ok((
                StatusCode::CREATED,
                Json(EnrollResponse {
                    agent_id: agent_id.to_string(),
                    tenant_id: tenant_id.to_string(),
                    certificate: STANDARD.encode(&issued.cert_der),
                    ca_chain: issued
                        .chain_der
                        .iter()
                        .map(|der| STANDARD.encode(der))
                        .collect(),
                    certificate_expires_at: format_time(issued.not_after),
                    signing_key_update: STANDARD.encode(&key_update_bytes),
                    signing_keys,
                    signing_keys_signature: STANDARD.encode(&signing_keys_signature),
                    grpc_endpoint: state.agent_endpoint().map(str::to_owned),
                }),
            ))
        }
    }
}

/// Rejects a request over the per-IP or the per-token-prefix rate (FR-ID-05, AC-ID-05).
fn limit(state: &AppState, source_ip: std::net::IpAddr, token: &str) -> Result<(), ApiError> {
    let instant = std::time::Instant::now();
    if let Err(wait) = state
        .auth()
        .enroll_ip_limiter
        .check(&crate::rate_limit::ip_key(source_ip), instant)
    {
        return Err(ApiError::rate_limited(wait.as_secs()));
    }
    let key = crate::rate_limit::enrollment_token_key(&token_prefix(token));
    if let Err(wait) = state.auth().enroll_token_limiter.check(&key, instant) {
        return Err(ApiError::rate_limited(wait.as_secs()));
    }
    Ok(())
}

/// The first [`TOKEN_PREFIX_LEN`] characters after the `re_` prefix, or the whole value.
fn token_prefix(token: &str) -> String {
    token
        .strip_prefix(crate::token::ENROLLMENT_TOKEN_PREFIX)
        .unwrap_or(token)
        .chars()
        .take(TOKEN_PREFIX_LEN)
        .collect()
}

/// Decodes the base64 DER of the request. A malformed value gives HTTP 400 on the field `csr`.
fn decode_csr(value: &str) -> Result<Vec<u8>, ApiError> {
    STANDARD.decode(value).map_err(|error| {
        ApiError::invalid_field_cause("csr", Reason::InvalidFormat, error.to_string())
    })
}

/// A bad request or a bad request body of the certificate signing request is a client error. Every
/// other cause (a missing issuing CA, a random failure) is internal.
fn map_ca_error(error: CaError) -> ApiError {
    match error {
        CaError::BadCsr(source) => {
            ApiError::invalid_field_cause("csr", Reason::InvalidFormat, source.to_string())
        }
        CaError::CsrCaFlag
        | CaError::CsrExtraSan
        | CaError::CsrKeyAlgorithm
        | CaError::BadSan { .. } => ApiError::invalid_field("csr", Reason::Unknown),
        other => ApiError::internal(other.to_string()),
    }
}

/// The one refusal that the client sees for an unknown, a used, an expired or a revoked token.
///
/// The body and the status are the same in every case, so the endpoint is no oracle (AC-IN-02).
fn refused() -> ApiError {
    ApiError::invalid_field("token", Reason::Unknown)
}

async fn write_refusal(
    tx: &mut TenantTx<'_>,
    token_id: EnrollmentTokenId,
    reason: EnrollmentRefusal,
) -> Result<(), restichub_db::DbError> {
    let mut event = AuditEvent::new(AuditAction::EnrollmentTokenRefuse, "enrollment_token");
    event.object_id = Some(token_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"reason": reason.as_str()})),
        &[],
    ));
    restichub_db::audit::insert(tx, &event).await?;
    Ok(())
}

async fn write_use(
    tx: &mut TenantTx<'_>,
    token_id: EnrollmentTokenId,
    agent_id: AgentId,
    source_ip: std::net::IpAddr,
) -> Result<(), restichub_db::DbError> {
    let mut event = AuditEvent::new(AuditAction::EnrollmentTokenUse, "enrollment_token");
    event.object_id = Some(token_id.as_uuid());
    event.source_ip = Some(source_ip);
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"agent_id": agent_id.to_string()})),
        &[],
    ));
    restichub_db::audit::insert(tx, &event).await?;
    Ok(())
}

/// The host facts of the registry row and the audit record.
struct EnrollFacts<'a> {
    hostname: &'a str,
    os: &'a str,
    arch: &'a str,
    agent_version: &'a str,
    restic_version: &'a str,
    labels: &'a [String],
    serial: &'a [u8],
    not_after: OffsetDateTime,
}

async fn write_enroll(
    tx: &mut TenantTx<'_>,
    agent_id: AgentId,
    source_ip: std::net::IpAddr,
    facts: &EnrollFacts<'_>,
) -> Result<(), restichub_db::DbError> {
    let mut event = AuditEvent::new(AuditAction::AgentEnroll, "agent");
    event.actor = Actor::Agent(agent_id.as_uuid());
    event.source_ip = Some(source_ip);
    event.object_id = Some(agent_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "hostname": facts.hostname,
            "os": facts.os,
            "arch": facts.arch,
            "agent_version": facts.agent_version,
            "restic_version": facts.restic_version,
            "labels": facts.labels,
            "certificate_serial": to_hex(facts.serial),
            "certificate_expires_at": format_time(facts.not_after),
        })),
        &[],
    ));
    restichub_db::audit::insert(tx, &event).await?;
    Ok(())
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
