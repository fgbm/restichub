//! The enrollment client of the agent (ADR-0026, FR-ID-02, ADR-0032).
//!
//! `register` builds an ECDSA P-256 key and a certificate signing request, sends them with the
//! one-time token over HTTPS and reads the certificate, the CA chain and the signed job signing key
//! set. The client verifies the control plane with the system roots or with the CA from `--ca-file`
//! and never disables verification (ADR-0032).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::header;
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{CertificateDer, ServerName};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::machine_id::MachineId;
use crate::pem;

/// The enrollment route of the agent listener (ADR-0026).
pub const ENROLL_PATH: &str = "/agent/v1/enroll";

/// Time budget for the TCP connect, the TLS handshake and the whole request.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Upper bound of the response body, so a hostile peer cannot exhaust the memory of the agent.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Host details that the agent reports at enrollment (FR-IN-07).
#[derive(Debug, Clone)]
pub struct Facts {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
    pub restic_version: String,
    /// Sent as a hint for the duplicate warning; the control plane stores only a hash.
    pub machine_id: Option<MachineId>,
}

/// The material that the control plane returns at enrollment (ADR-0026).
#[derive(Debug)]
pub struct Enrolled {
    pub agent_id: String,
    pub tenant_id: String,
    pub certificate_der: Vec<u8>,
    /// Issuing CA first, root last.
    pub ca_chain_der: Vec<Vec<u8>>,
    pub signing_key_update: Vec<u8>,
    /// The public job signing key set: the current key and the next key (ADR-0026).
    pub signing_keys: Vec<EnrollmentKeyBody>,
    /// Ed25519 signature over the key set, base64 (standard alphabet).
    pub signing_keys_signature: String,
    pub grpc_endpoint: Option<String>,
    /// The token was bound to a host: the control plane replaced the identity of that host.
    pub reenrolled: bool,
}

/// One public job signing key of the enrollment response (ADR-0026).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentKeyBody {
    /// `current` or `next`.
    pub role: String,
    pub key_id: String,
    /// Base64 of the 32 Ed25519 bytes (standard alphabet).
    pub public_key: String,
}

/// Speaks to the enrollment endpoint over verified HTTPS (ADR-0032).
pub struct Client {
    tls: Arc<ClientConfig>,
}

impl Client {
    /// Builds the client. A `ca_file` replaces the system roots with the CA in the PEM file.
    ///
    /// # Errors
    ///
    /// Returns an error when the CA file or the system roots hold no usable certificate.
    pub fn new(ca_file: Option<&Path>) -> Result<Self> {
        // A single crypto provider keeps `ClientConfig::builder` deterministic.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let roots = match ca_file {
            Some(path) => roots_from_file(path)?,
            None => system_roots()?,
        };
        let tls = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self { tls: Arc::new(tls) })
    }

    /// Sends the certificate signing request and the token to the control plane (FR-IN-07).
    ///
    /// # Errors
    ///
    /// Returns an error on a network failure, a TLS failure, a refusal or a rate limit.
    pub async fn enroll(
        &self,
        url: &str,
        token: &str,
        facts: &Facts,
        csr_der: &[u8],
    ) -> Result<Enrolled> {
        let origin = Origin::parse(url)?;
        let request = EnrollRequest {
            token,
            csr: STANDARD.encode(csr_der),
            hostname: &facts.hostname,
            os: &facts.os,
            arch: &facts.arch,
            agent_version: &facts.agent_version,
            restic_version: &facts.restic_version,
            machine_id: facts.machine_id.as_ref().map(MachineId::as_str),
        };
        let body = serde_json::to_vec(&request).context("cannot serialize the request")?;
        let response = self.post(&origin, hyper::Method::POST, body).await?;
        match response.status {
            201 => parse_response(&response.body),
            429 => bail!(
                "the control plane is rate limiting the enrollment{}",
                retry_suffix(response.retry_after)
            ),
            status => bail!(
                "the control plane refused the enrollment: HTTP {status} {}; {}",
                describe_problem(&response.body),
                refusal_hint(status)
            ),
        }
    }

    /// Reads the clock of the control plane from the `Date` header of an unauthenticated request.
    ///
    /// The request is a `GET` on the enrollment route. The route answers it with a refusal, never
    /// with an enrollment, so no token is used. The answer is `None` when the request fails or the
    /// header is absent or unreadable; the enrollment itself reports a network failure.
    pub async fn server_time(&self, url: &str) -> Option<OffsetDateTime> {
        let origin = Origin::parse(url).ok()?;
        let response = self
            .post(&origin, hyper::Method::GET, Vec::new())
            .await
            .ok()?;
        OffsetDateTime::parse(response.date.as_deref()?, &Rfc2822).ok()
    }

    async fn post(
        &self,
        origin: &Origin,
        method: hyper::Method,
        body: Vec<u8>,
    ) -> Result<HttpResponse> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.request(origin, method, body))
            .await
            .map_err(|_| anyhow::anyhow!("the enrollment request timed out"))?
    }

    async fn request(
        &self,
        origin: &Origin,
        method: hyper::Method,
        body: Vec<u8>,
    ) -> Result<HttpResponse> {
        let tcp = tokio::time::timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect((origin.host.as_str(), origin.port)),
        )
        .await
        .map_err(|_| anyhow::anyhow!("the connection to the control plane timed out"))?
        .with_context(|| format!("cannot connect to {}:{}", origin.host, origin.port))?;
        let connector = TlsConnector::from(self.tls.clone());
        let server_name = ServerName::try_from(origin.host.clone()).map_err(|error| {
            anyhow::anyhow!("`{}` is not a valid server name: {error}", origin.host)
        })?;
        let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(server_name, tcp))
            .await
            .map_err(|_| anyhow::anyhow!("the TLS handshake with the control plane timed out"))?
            .context("the TLS handshake with the control plane failed")?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .context("cannot start the HTTP connection")?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        let request = hyper::Request::builder()
            .method(method)
            .uri(ENROLL_PATH)
            .header(header::HOST, &origin.authority)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .body(Full::new(Bytes::from(body)))
            .context("cannot build the request")?;
        let response = sender
            .send_request(request)
            .await
            .context("cannot send the request")?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let date = response
            .headers()
            .get(header::DATE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
            .collect()
            .await
            .map_err(|error| {
                anyhow::anyhow!("the response body is too large or unreadable: {error}")
            })?
            .to_bytes()
            .to_vec();
        // The connection stays open after the response, so the task ends only when the sender drops.
        drop(sender);
        let _ = task.await;
        Ok(HttpResponse {
            status,
            retry_after,
            date,
            body,
        })
    }
}

struct HttpResponse {
    status: u16,
    retry_after: Option<u64>,
    date: Option<String>,
    body: Vec<u8>,
}

#[derive(Serialize)]
struct EnrollRequest<'a> {
    token: &'a str,
    csr: String,
    hostname: &'a str,
    os: &'a str,
    arch: &'a str,
    agent_version: &'a str,
    restic_version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    machine_id: Option<&'a str>,
}

#[derive(Deserialize)]
struct EnrollResponse {
    agent_id: String,
    tenant_id: String,
    certificate: String,
    ca_chain: Vec<String>,
    signing_key_update: String,
    signing_keys: Vec<EnrollmentKeyBody>,
    signing_keys_signature: String,
    grpc_endpoint: Option<String>,
    #[serde(default)]
    reenrolled: bool,
}

#[derive(Deserialize)]
struct Problem {
    title: String,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

fn parse_response(body: &[u8]) -> Result<Enrolled> {
    let response: EnrollResponse =
        serde_json::from_slice(body).context("the enrollment response is not valid JSON")?;
    let ca_chain_der = response
        .ca_chain
        .iter()
        .map(|entry| {
            STANDARD
                .decode(entry)
                .context("a chain entry is not base64")
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Enrolled {
        agent_id: response.agent_id,
        tenant_id: response.tenant_id,
        certificate_der: STANDARD
            .decode(&response.certificate)
            .context("the certificate is not base64")?,
        ca_chain_der,
        signing_key_update: STANDARD
            .decode(&response.signing_key_update)
            .context("the key update is not base64")?,
        signing_keys: response.signing_keys,
        signing_keys_signature: response.signing_keys_signature,
        grpc_endpoint: response.grpc_endpoint,
        reenrolled: response.reenrolled,
    })
}

/// What the person does after a refusal. An enrollment token works once and expires.
fn refusal_hint(status: u16) -> &'static str {
    match status {
        400 | 401 | 403 | 404 | 409 | 410 => {
            "create a new enrollment token in the console and run register again"
        }
        500..=599 => "the control plane has an error; wait a moment and run register again",
        _ => "check the URL and run register again",
    }
}

fn describe_problem(body: &[u8]) -> String {
    let Ok(problem) = serde_json::from_slice::<Problem>(body) else {
        return "with an unreadable body".to_owned();
    };
    match (&problem.field, &problem.reason) {
        (Some(field), Some(reason)) => format!("{} ({field}: {reason})", problem.title),
        _ => problem.title,
    }
}

fn retry_suffix(retry_after: Option<u64>) -> String {
    match retry_after {
        Some(seconds) => format!("; retry after {seconds} s"),
        None => String::new(),
    }
}

fn roots_from_file(path: &Path) -> Result<RootCertStore> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the CA file {}", path.display()))?;
    let mut roots = RootCertStore::empty();
    let mut added = 0;
    for der in pem::decode_all(&text)? {
        roots
            .add(CertificateDer::from(der))
            .context("the CA file holds a bad certificate")?;
        added += 1;
    }
    if added == 0 {
        bail!("the CA file {} holds no certificate", path.display());
    }
    Ok(roots)
}

pub(crate) fn system_roots() -> Result<RootCertStore> {
    let result = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    for cert in result.certs {
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        bail!("the system holds no trusted root certificate; give --ca-file");
    }
    Ok(roots)
}

struct Origin {
    host: String,
    port: u16,
    authority: String,
}

impl Origin {
    fn parse(url: &str) -> Result<Self> {
        let rest = url
            .strip_prefix("https://")
            .context("the URL must start with `https://`")?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        if authority.is_empty() {
            bail!("the control plane URL holds no host");
        }
        let (host, port) = split_authority(authority)?;
        Ok(Self {
            host,
            port,
            authority: authority.to_owned(),
        })
    }
}

pub(crate) fn split_authority(authority: &str) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .context("the IPv6 address is not closed")?;
        let port = match after.strip_prefix(':') {
            Some(port) => port.parse().context("the port is not a number")?,
            None => 443,
        };
        return Ok((host.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Ok((
            host.to_owned(),
            port.parse().context("the port is not a number")?,
        )),
        _ => Ok((authority.to_owned(), 443)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn an_https_url_gives_the_host_and_the_port() {
        let origin = Origin::parse("https://cp.example:8443").expect("parse");
        assert_eq!(origin.host, "cp.example");
        assert_eq!(origin.port, 8443);
        assert_eq!(origin.authority, "cp.example:8443");
    }

    #[test]
    fn a_url_without_a_port_uses_443() {
        let origin = Origin::parse("https://cp.example").expect("parse");
        assert_eq!(origin.port, 443);
    }

    #[test]
    fn a_path_is_ignored() {
        let origin = Origin::parse("https://cp.example:8443/base").expect("parse");
        assert_eq!(origin.host, "cp.example");
        assert_eq!(origin.port, 8443);
    }

    #[test]
    fn an_ipv6_url_is_parsed() {
        let origin = Origin::parse("https://[2001:db8::1]:8443").expect("parse");
        assert_eq!(origin.host, "2001:db8::1");
        assert_eq!(origin.port, 8443);
    }

    #[test]
    fn a_refusal_tells_what_to_do() {
        assert!(refusal_hint(409).contains("new enrollment token"));
        assert!(refusal_hint(503).contains("run register again"));
        assert!(refusal_hint(418).contains("URL"));
    }

    #[test]
    fn a_plain_http_url_is_refused() {
        assert!(Origin::parse("http://cp.example").is_err());
    }
}
