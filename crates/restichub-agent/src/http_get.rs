//! A bounded HTTPS client for the restic download (ADR-0038).
//!
//! It speaks HTTPS only and always verifies the server certificate. It follows a limited number of
//! redirects, and it honours `HTTPS_PROXY` through a `CONNECT` tunnel. Every read has a size bound
//! and every step has a time bound.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http_body_util::{BodyExt as _, Empty, Limited};
use hyper::header;
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::enroll::{split_authority, system_roots};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_REDIRECTS: usize = 5;
const MAX_PROXY_HEAD_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Url {
    host: String,
    port: u16,
    authority: String,
    target: String,
}

impl Url {
    fn parse(url: &str) -> Result<Self> {
        let rest = url
            .strip_prefix("https://")
            .with_context(|| format!("`{url}` is not an https:// URL"))?;
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(split);
        if authority.is_empty() || authority.contains('@') {
            bail!("`{url}` holds no host or holds credentials");
        }
        let (host, port) = split_authority(authority)?;
        let tail = tail.split('#').next().unwrap_or_default();
        let target = if tail.is_empty() {
            "/".to_owned()
        } else if tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_owned()
        };
        Ok(Self {
            host,
            port,
            authority: authority.to_owned(),
            target,
        })
    }

    fn redirect(&self, location: &str) -> Result<Self> {
        if location.starts_with("https://") {
            Self::parse(location)
        } else if location.starts_with('/') && !location.starts_with("//") {
            Self::parse(&format!("https://{}{location}", self.authority))
        } else {
            bail!("the redirect to `{location}` is not an https:// URL")
        }
    }
}

/// `CONNECT` tunnel settings from `HTTPS_PROXY` and `NO_PROXY` (ADR-0038).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    host: String,
    port: u16,
    authorization: Option<String>,
    bypass: Vec<String>,
}

impl Proxy {
    /// Reads `HTTPS_PROXY` (or `https_proxy`) and `NO_PROXY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the proxy URL is not an `http://` URL.
    pub fn from_env() -> Result<Option<Self>> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        let value = ["HTTPS_PROXY", "https_proxy"]
            .into_iter()
            .find_map(|name| lookup(name).filter(|value| !value.trim().is_empty()));
        let Some(value) = value else {
            return Ok(None);
        };
        let bypass = ["NO_PROXY", "no_proxy"]
            .into_iter()
            .find_map(&lookup)
            .unwrap_or_default()
            .split(',')
            .map(|entry| entry.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|entry| !entry.is_empty())
            .collect();
        Self::parse(&value, bypass).map(Some)
    }

    /// Reads an `http://[user:password@]host[:port]` proxy URL.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is not an `http://` URL or holds no host.
    pub fn parse(value: &str, bypass: Vec<String>) -> Result<Self> {
        let rest = value
            .trim()
            .strip_prefix("http://")
            .context("HTTPS_PROXY must be an http:// URL of the proxy")?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        let (credentials, host_port) = match authority.rsplit_once('@') {
            Some((credentials, host_port)) => (Some(credentials), host_port),
            None => (None, authority),
        };
        if host_port.is_empty() {
            bail!("HTTPS_PROXY holds no host");
        }
        let (host, port) = proxy_host_port(host_port)?;
        let authorization = credentials
            .map(|credentials| {
                // The userinfo of a URL is percent-encoded, and the Basic scheme needs the raw bytes.
                let decoded = percent_decode(credentials)?;
                Ok::<_, anyhow::Error>(format!("Basic {}", STANDARD.encode(decoded)))
            })
            .transpose()?;
        Ok(Self {
            host,
            port,
            authorization,
            bypass,
        })
    }

    fn applies_to(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        !self
            .bypass
            .iter()
            .any(|entry| entry == "*" || host == *entry || host.ends_with(&format!(".{entry}")))
    }
}

/// Splits `host`, `host:port`, `[v6]` and `[v6]:port`. The default port is 80.
fn proxy_host_port(host_port: &str) -> Result<(String, u16)> {
    let parse_port =
        |port: &str| -> Result<u16> { port.parse().context("the proxy port is not a number") };
    if let Some(rest) = host_port.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .context("the proxy address has an open bracket")?;
        if host.is_empty() {
            bail!("HTTPS_PROXY holds no host");
        }
        let port = match tail {
            "" => 80,
            tail => parse_port(
                tail.strip_prefix(':')
                    .context("the proxy address has text after the bracket")?,
            )?,
        };
        return Ok((host.to_owned(), port));
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Ok((host.to_owned(), parse_port(port)?)),
        _ => Ok((host_port.to_owned(), 80)),
    }
}

fn percent_decode(text: &str) -> Result<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let digits = bytes
                .get(index + 1..index + 3)
                .and_then(|pair| std::str::from_utf8(pair).ok())
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .context("the proxy credentials hold a bad percent escape")?;
            out.push(digits);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Ok(out)
}

/// The final answer after the redirects, with any status.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: hyper::HeaderMap,
    pub body: Vec<u8>,
}

/// Speaks to a host over verified HTTPS (ADR-0038).
pub struct HttpsClient {
    tls: Arc<ClientConfig>,
    proxy: Option<Proxy>,
}

impl HttpsClient {
    /// Builds the client with the system roots and the proxy from the environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the system holds no trusted root or the proxy setting is bad.
    pub fn from_system() -> Result<Self> {
        Ok(Self::new(system_roots()?, Proxy::from_env()?))
    }

    /// Builds the client with explicit roots and an explicit proxy.
    #[must_use]
    pub fn new(roots: RootCertStore, proxy: Option<Proxy>) -> Self {
        // A single crypto provider keeps `ClientConfig::builder` deterministic.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self {
            tls: Arc::new(tls),
            proxy,
        }
    }

    /// Fetches a URL and follows up to five redirects. A body above `max_bytes` is an error.
    ///
    /// The result is the last response, whatever its status.
    ///
    /// # Errors
    ///
    /// Returns an error on a network failure, a TLS failure, a timeout, a redirect to a URL that is
    /// not `https://`, or a body that is too large.
    pub async fn get(&self, url: &str, accept: &str, max_bytes: usize) -> Result<Response> {
        let mut current = Url::parse(url)?;
        for _ in 0..=MAX_REDIRECTS {
            let response =
                tokio::time::timeout(REQUEST_TIMEOUT, self.get_once(&current, accept, max_bytes))
                    .await
                    .map_err(|_| anyhow::anyhow!("the request to {} timed out", current.host))??;
            if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let location = response
                .headers
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .context("a redirect holds no Location header")?;
            current = current.redirect(location)?;
        }
        bail!("the request to {url} was redirected too many times")
    }

    async fn get_once(&self, url: &Url, accept: &str, max_bytes: usize) -> Result<Response> {
        let tcp = self.connect_tcp(url).await?;
        let connector = TlsConnector::from(self.tls.clone());
        let server_name = ServerName::try_from(url.host.clone()).map_err(|error| {
            anyhow::anyhow!("`{}` is not a valid server name: {error}", url.host)
        })?;
        let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(server_name, tcp))
            .await
            .map_err(|_| anyhow::anyhow!("the TLS handshake with {} timed out", url.host))?
            .with_context(|| format!("the TLS handshake with {} failed", url.host))?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .context("cannot start the HTTP connection")?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        let request = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri(&url.target)
            .header(header::HOST, &url.authority)
            .header(header::ACCEPT, accept)
            .header(
                header::USER_AGENT,
                &format!("restichub-agent/{}", restichub_common::BUILD_VERSION),
            )
            .body(Empty::<Bytes>::new())
            .context("cannot build the request")?;
        let response = sender
            .send_request(request)
            .await
            .context("cannot send the request")?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = Limited::new(response.into_body(), max_bytes)
            .collect()
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "the response from {} is too large or unreadable: {error}",
                    url.host
                )
            })?
            .to_bytes()
            .to_vec();
        drop(sender);
        let _ = task.await;
        Ok(Response {
            status,
            headers,
            body,
        })
    }

    async fn connect_tcp(&self, url: &Url) -> Result<TcpStream> {
        let via = self
            .proxy
            .as_ref()
            .filter(|proxy| proxy.applies_to(&url.host));
        let (host, port) = via.map_or((url.host.as_str(), url.port), |proxy| {
            (proxy.host.as_str(), proxy.port)
        });
        let mut tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
            .await
            .map_err(|_| anyhow::anyhow!("the connection to {host}:{port} timed out"))?
            .with_context(|| format!("cannot connect to {host}:{port}"))?;
        if let Some(proxy) = via {
            tokio::time::timeout(CONNECT_TIMEOUT, open_tunnel(&mut tcp, proxy, url))
                .await
                .map_err(|_| anyhow::anyhow!("the proxy {host}:{port} did not answer in time"))??;
        }
        Ok(tcp)
    }
}

async fn open_tunnel<S>(stream: &mut S, proxy: &Proxy, url: &Url) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let target = format!("{}:{}", url.host, url.port);
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(authorization) = &proxy.authorization {
        request.push_str(&format!("Proxy-Authorization: {authorization}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .context("cannot write to the proxy")?;
    // The tunnel carries TLS after the head, so the read stops exactly at the blank line.
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_PROXY_HEAD_BYTES {
            bail!("the proxy answer is too long");
        }
        stream
            .read_exact(&mut byte)
            .await
            .context("the proxy closed the connection")?;
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head);
    let status = text.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("200") {
        bail!("the proxy refused the tunnel to {target}: {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn an_https_url_is_split_into_host_port_and_target() {
        let url = Url::parse("https://api.example:8443/repos/x?y=1#frag").expect("url");
        assert_eq!(url.host, "api.example");
        assert_eq!(url.port, 8443);
        assert_eq!(url.target, "/repos/x?y=1");
    }

    #[test]
    fn a_url_without_a_path_gets_a_root_target() {
        assert_eq!(Url::parse("https://a.example").expect("url").target, "/");
        assert_eq!(
            Url::parse("https://a.example?x=1").expect("url").target,
            "/?x=1"
        );
    }

    #[test]
    fn plain_http_and_credentials_are_refused() {
        assert!(Url::parse("http://a.example/x").is_err());
        assert!(Url::parse("https://user:pw@a.example/x").is_err());
    }

    #[test]
    fn a_redirect_must_stay_on_https() {
        let base = Url::parse("https://a.example/x").expect("url");
        assert!(base.redirect("http://b.example/y").is_err());
        assert!(base.redirect("//b.example/y").is_err());
        assert_eq!(base.redirect("/z").expect("path").authority, "a.example");
        assert_eq!(
            base.redirect("https://b.example/y").expect("absolute").host,
            "b.example"
        );
    }

    #[test]
    fn the_proxy_is_read_from_the_environment_values() {
        let proxy = Proxy::from_lookup(|name| match name {
            "HTTPS_PROXY" => Some("http://user:pw@proxy.example:3128".to_owned()),
            "NO_PROXY" => Some("localhost, .internal.example".to_owned()),
            _ => None,
        })
        .expect("proxy")
        .expect("set");
        assert_eq!(proxy.host, "proxy.example");
        assert_eq!(proxy.port, 3128);
        assert!(proxy.authorization.is_some());
        assert!(proxy.applies_to("github.com"));
        assert!(!proxy.applies_to("localhost"));
        assert!(!proxy.applies_to("a.internal.example"));
    }

    #[test]
    fn a_proxy_user_info_is_percent_decoded() {
        let proxy =
            Proxy::parse("http://us%40er:p%3Aw%25@proxy.example:3128", Vec::new()).expect("proxy");
        let expected = format!("Basic {}", STANDARD.encode("us@er:p:w%"));
        assert_eq!(proxy.authorization.as_deref(), Some(expected.as_str()));
        assert!(Proxy::parse("http://u:%zz@proxy.example", Vec::new()).is_err());
    }

    #[test]
    fn a_proxy_address_in_brackets_is_read() {
        let proxy = Proxy::parse("http://[2001:db8::1]:3128", Vec::new()).expect("proxy");
        assert_eq!((proxy.host.as_str(), proxy.port), ("2001:db8::1", 3128));
        let default_port = Proxy::parse("http://[::1]", Vec::new()).expect("proxy");
        assert_eq!((default_port.host.as_str(), default_port.port), ("::1", 80));
        assert!(Proxy::parse("http://[::1", Vec::new()).is_err());
        assert!(Proxy::parse("http://[::1]x", Vec::new()).is_err());
    }

    #[test]
    fn no_proxy_variable_means_no_proxy() {
        assert!(Proxy::from_lookup(|_| None).expect("proxy").is_none());
    }

    #[test]
    fn an_https_proxy_url_is_refused() {
        assert!(Proxy::parse("https://proxy.example", Vec::new()).is_err());
    }
}
