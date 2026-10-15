//! The enrollment client of the emulated agent (ADR-0026, epic #1).
//!
//! The real agent enrolls with `restichub-agent register` (ADR-0032). The emulator uses this
//! minimal client to enroll one agent against the control plane in an integration test: it builds
//! an ECDSA P-256 key and a certificate signing request, sends the JSON request of
//! `POST /agent/v1/enroll` over HTTP/1.1 and reads the certificate, the CA chain and the signed job
//! signing key set.
//!
//! The endpoint is HTTPS in production (ADR-0026). [`enroll_tls`] speaks HTTPS with the installation
//! root CA; [`enroll`] speaks plain HTTP, so a test can drive the handler without TLS. The emulator
//! keeps the request small; the real client of the agent crate arrives with the agent (epic #1).

use anyhow::{Context, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use serde_json::json;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

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

/// The TLS trust material of an HTTPS enrollment (ADR-0026).
#[derive(Debug, Clone)]
pub struct EnrollTls {
    /// Root certificate authority of the installation, in DER.
    pub ca_der: Vec<u8>,
    /// The DNS name in the certificate that the listener presents.
    pub server_name: String,
}

/// Sends the enrollment request of one agent to `base_url` with `token` over plain HTTP.
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
    enroll_with(base_url, token, facts, None).await
}

/// Sends the enrollment request of one agent over HTTPS with the installation root CA (ADR-0026).
///
/// # Errors
///
/// Returns an error when the TLS handshake, the request or the response fails.
pub async fn enroll_tls(
    base_url: &str,
    token: &str,
    facts: &EnrollFacts,
    tls: &EnrollTls,
) -> anyhow::Result<EnrolledIdentity> {
    enroll_with(base_url, token, facts, Some(tls)).await
}

async fn enroll_with(
    base_url: &str,
    token: &str,
    facts: &EnrollFacts,
    tls: Option<&EnrollTls>,
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

    let response = post_json(base_url, ENROLL_PATH, &body, None, tls).await?;
    parse_response(&response, key.serialize_pem())
}

/// Creates an enrollment token through the public API with an API token and returns its value
/// (FR-ID-01). The API listener may be plain HTTP or HTTPS with the installation root CA.
///
/// # Errors
///
/// Returns an error when the request fails, the control plane refuses it, or the response holds no
/// token.
pub async fn create_enrollment_token(
    api_url: &str,
    api_token: &str,
    ttl_seconds: i64,
    tls: Option<&EnrollTls>,
) -> anyhow::Result<String> {
    let body = serde_json::to_vec(&json!({ "ttl_seconds": ttl_seconds }))
        .context("cannot serialize the request")?;
    let response = post_json(
        api_url,
        "/api/v1/enrollment-tokens",
        &body,
        Some(api_token),
        tls,
    )
    .await?;
    let value: serde_json::Value =
        serde_json::from_slice(&response).context("the response is not JSON")?;
    value
        .get("token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .context("the response holds no `token`")
}

async fn post_json(
    base_url: &str,
    path: &str,
    body: &[u8],
    bearer: Option<&str>,
    tls: Option<&EnrollTls>,
) -> anyhow::Result<Vec<u8>> {
    let (host, port, https) = split_origin(base_url)?;
    match (https, tls) {
        (true, Some(tls)) => post_tls(&host, port, path, body, bearer, tls).await,
        (true, None) => bail!("an https base URL needs the TLS trust material"),
        (false, _) => post(&host, port, path, body, bearer).await,
    }
}

fn request_head(host: &str, path: &str, length: usize, bearer: Option<&str>) -> String {
    let authorization = bearer
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\n{authorization}Content-Type: application/json\r\n\
         Content-Length: {length}\r\nConnection: close\r\n\r\n"
    )
}

/// Sends one HTTP/1.1 `POST` and returns the response body on HTTP 201.
async fn post(
    host: &str,
    port: u16,
    path: &str,
    body: &[u8],
    bearer: Option<&str>,
) -> anyhow::Result<Vec<u8>> {
    let mut stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("cannot connect to {host}:{port}"))?;
    let head = request_head(host, path, body.len(), bearer);
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

/// Sends one HTTP/1.1 `POST` over TLS and returns the response body on HTTP 201.
async fn post_tls(
    host: &str,
    port: u16,
    path: &str,
    body: &[u8],
    bearer: Option<&str>,
    tls: &EnrollTls,
) -> anyhow::Result<Vec<u8>> {
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates([CertificateDer::from(tls.ca_der.clone())]);
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&tokio_rustls::rustls::version::TLS13])
        .context("cannot build the TLS client configuration")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name =
        ServerName::try_from(tls.server_name.clone()).context("the server name is not valid")?;
    let stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("cannot connect to {host}:{port}"))?;
    let mut stream = connector
        .connect(server_name, stream)
        .await
        .context("the TLS handshake failed")?;
    let head = request_head(host, path, body.len(), bearer);
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

/// Splits `http://host:port` or `https://host:port` into the host, the port and the scheme. A path
/// in the URL is ignored.
fn split_origin(base_url: &str) -> anyhow::Result<(String, u16, bool)> {
    let (rest, https, default_port) = if let Some(rest) = base_url.strip_prefix("https://") {
        (rest, true, 443)
    } else if let Some(rest) = base_url.strip_prefix("http://") {
        (rest, false, 80)
    } else {
        bail!("the base URL must start with `http://` or `https://`");
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>().context("the port is not a number")?,
        ),
        None => (authority, default_port),
    };
    Ok((host.to_owned(), port, https))
}
