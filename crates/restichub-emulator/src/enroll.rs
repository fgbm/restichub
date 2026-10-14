//! The enrollment client of the emulated agent (ADR-0026, epic #1).
//!
//! The real agent enrolls with `restichub-agent register` (ADR-0032). The emulator uses this
//! minimal client to enroll one agent against the control plane in an integration test: it builds
//! an ECDSA P-256 key and a certificate signing request, sends the JSON request of
//! `POST /agent/v1/enroll` over HTTP/1.1 and reads the certificate, the CA chain and the signed job
//! signing key set.
//!
//! The endpoint is HTTPS in production. This client speaks plain HTTP, so the integration test runs
//! the control plane without TLS. The emulator keeps the request small; the real client of the agent
//! crate arrives with the agent (epic #1).

use anyhow::{Context, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The host facts that an agent reports at enrollment.
#[derive(Debug, Clone)]
pub struct EnrollFacts {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
    pub restic_version: String,
}

/// The certificate and key material that the control plane returns at enrollment.
#[derive(Debug, Clone)]
pub struct EnrolledIdentity {
    pub agent_id: String,
    pub tenant_id: String,
    pub certificate_der: Vec<u8>,
    /// The issuing CA first and the root last, so a client verifies the chain in order.
    pub ca_chain_der: Vec<Vec<u8>>,
    pub key_pem: String,
    pub signing_key_update: Vec<u8>,
    /// The public job signing key set: the current key and the next key (ADR-0026).
    pub signing_keys: Vec<EnrollmentKeyBody>,
    /// Ed25519 signature over the key set, base64 (standard alphabet).
    pub signing_keys_signature: String,
    pub grpc_endpoint: Option<String>,
}

/// One public job signing key of the enrollment response (ADR-0026).
#[derive(Debug, Clone)]
pub struct EnrollmentKeyBody {
    /// `current` or `next`.
    pub role: String,
    pub key_id: String,
    /// Base64 of the 32 Ed25519 bytes (standard alphabet).
    pub public_key: String,
}

/// The enrollment route of the control plane (ADR-0026).
pub const ENROLL_PATH: &str = "/agent/v1/enroll";

/// Sends the enrollment request of one agent to `base_url` with `token`.
///
/// # Errors
///
/// Returns an error when the key, the request or the response is not valid, or when the control
/// plane refuses the enrollment.
pub async fn enroll(
    base_url: &str,
    token: &str,
    facts: &EnrollFacts,
) -> anyhow::Result<EnrolledIdentity> {
    let key =
        KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).context("cannot generate the agent key")?;
    let csr = CertificateParams::default()
        .serialize_request(&key)
        .context("cannot build the certificate signing request")?;
    let body = json!({
        "token": token,
        "csr": STANDARD.encode(csr.der()),
        "hostname": facts.hostname,
        "os": facts.os,
        "arch": facts.arch,
        "agent_version": facts.agent_version,
        "restic_version": facts.restic_version,
    });
    let body = serde_json::to_vec(&body).context("cannot serialize the request")?;

    let (host, port) = split_origin(base_url)?;
    let response = post(&host, port, ENROLL_PATH, &body).await?;
    parse_response(&response, key.serialize_pem())
}

/// Sends one HTTP/1.1 `POST` and returns the response body on HTTP 201.
async fn post(host: &str, port: u16, path: &str, body: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("cannot connect to {host}:{port}"))?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .context("cannot send the request head")?;
    stream
        .write_all(body)
        .await
        .context("cannot send the request body")?;
    stream.flush().await.context("cannot flush the request")?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .context("cannot read the response")?;
    split_response(&response)
}

/// Splits an HTTP/1.1 response and returns the body of HTTP 201. Another status is an error.
fn split_response(response: &[u8]) -> anyhow::Result<Vec<u8>> {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .context("the response holds no header terminator")?;
    let head = std::str::from_utf8(&response[..split]).context("the response head is not text")?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .context("the response holds no status")?;
    let body = response[split + 4..].to_vec();
    if status != 201 {
        bail!(
            "the enrollment was refused with HTTP {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    Ok(body)
}

fn parse_response(body: &[u8], key_pem: String) -> anyhow::Result<EnrolledIdentity> {
    let value: serde_json::Value =
        serde_json::from_slice(body).context("the response is not JSON")?;
    let text = |name: &str| -> anyhow::Result<String> {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .with_context(|| format!("the response holds no `{name}`"))
    };
    let der = |name: &str| -> anyhow::Result<Vec<u8>> {
        STANDARD
            .decode(text(name)?)
            .with_context(|| format!("the field `{name}` is not base64"))
    };
    let ca_chain_der = value
        .get("ca_chain")
        .and_then(serde_json::Value::as_array)
        .context("the response holds no `ca_chain`")?
        .iter()
        .map(|entry| {
            STANDARD
                .decode(entry.as_str().unwrap_or_default())
                .context("a chain entry is not base64")
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let signing_keys = value
        .get("signing_keys")
        .and_then(serde_json::Value::as_array)
        .context("the response holds no `signing_keys`")?
        .iter()
        .map(|entry| {
            Ok(EnrollmentKeyBody {
                role: entry
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .context("a key holds no `role`")?
                    .to_owned(),
                key_id: entry
                    .get("key_id")
                    .and_then(serde_json::Value::as_str)
                    .context("a key holds no `key_id`")?
                    .to_owned(),
                public_key: entry
                    .get("public_key")
                    .and_then(serde_json::Value::as_str)
                    .context("a key holds no `public_key`")?
                    .to_owned(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(EnrolledIdentity {
        agent_id: text("agent_id")?,
        tenant_id: text("tenant_id")?,
        certificate_der: der("certificate")?,
        ca_chain_der,
        key_pem,
        signing_key_update: der("signing_key_update")?,
        signing_keys,
        signing_keys_signature: text("signing_keys_signature")?,
        grpc_endpoint: value
            .get("grpc_endpoint")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

/// Splits `http://host:port` into the host and the port. A path in the URL is ignored. Only
/// `http` is accepted.
fn split_origin(base_url: &str) -> anyhow::Result<(String, u16)> {
    let rest = base_url
        .strip_prefix("http://")
        .context("the base URL must start with `http://`")?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>().context("the port is not a number")?,
        ),
        None => (authority, 80),
    };
    Ok((host.to_owned(), port))
}
