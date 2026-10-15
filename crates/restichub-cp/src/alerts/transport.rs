//! The parts that the email and webhook senders share: the stream type, the error code, the TLS
//! client configuration and the host name resolver (ADR-0035).

use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::pki_types::CertificateDer;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};

use super::message::scrub;
use super::ssrf::UrlError;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub type BoxIo = Box<dyn Io>;

/// Why a send failed. The text of [`SendError::code`] is stored in the delivery row and shown to
/// an Admin, so it holds no secret and no response body of a target.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("the installation has no SMTP server")]
    SmtpNotConfigured,
    #[error("the webhook URL is not valid")]
    InvalidUrl,
    #[error("the webhook target is a forbidden address")]
    BlockedAddress,
    #[error("the host name did not resolve")]
    DnsFailed,
    #[error("cannot connect")]
    ConnectFailed,
    #[error("the TLS handshake failed")]
    TlsFailed,
    #[error("the send took too long")]
    Timeout,
    #[error("the target answered HTTP {0}")]
    HttpStatus(u16),
    #[error("the answer is not valid")]
    BadResponse,
    #[error("the SMTP server answered {code}")]
    Smtp { code: u16, text: String },
    #[error("the SMTP server does not support {0}")]
    SmtpUnsupported(&'static str),
    #[error("the connection failed")]
    Io,
    #[error("the signing secret is not available")]
    SecretUnavailable,
    #[error("the channel is disabled")]
    ChannelDisabled,
}

impl SendError {
    /// A stable short text, at most 200 characters.
    pub fn code(&self) -> String {
        match self {
            Self::Smtp { code, text } => {
                format!("smtp_{code}: {}", scrub(text, 120))
            }
            Self::SmtpNotConfigured => "smtp_not_configured".to_owned(),
            Self::InvalidUrl => "invalid_url".to_owned(),
            Self::BlockedAddress => "blocked_address".to_owned(),
            Self::DnsFailed => "dns_failed".to_owned(),
            Self::ConnectFailed => "connect_failed".to_owned(),
            Self::TlsFailed => "tls_failed".to_owned(),
            Self::Timeout => "timeout".to_owned(),
            Self::HttpStatus(status) => format!("http_{status}"),
            Self::BadResponse => "bad_response".to_owned(),
            Self::SmtpUnsupported(feature) => format!("smtp_unsupported_{feature}"),
            Self::Io => "io_error".to_owned(),
            Self::SecretUnavailable => "secret_unavailable".to_owned(),
            Self::ChannelDisabled => "channel_disabled".to_owned(),
        }
    }

    /// True when a repeat cannot help: the same input fails the same way.
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::SmtpNotConfigured
                | Self::InvalidUrl
                | Self::BlockedAddress
                | Self::SecretUnavailable
                | Self::ChannelDisabled
        ) || matches!(self, Self::Smtp { code, .. } if (500..600).contains(code))
    }
}

impl From<UrlError> for SendError {
    fn from(error: UrlError) -> Self {
        match error {
            UrlError::Forbidden => Self::BlockedAddress,
            _ => Self::InvalidUrl,
        }
    }
}

impl From<io::Error> for SendError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

pub type ResolveFuture = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;

/// Looks up a host name. A test replaces it to answer with a chosen address.
pub trait Resolver: Send + Sync {
    fn resolve(&self, host: &str, port: u16) -> ResolveFuture;
}

#[derive(Debug, Default)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> ResolveFuture {
        let host = host.to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), port)).await?;
            Ok(addresses.map(|address| address.ip()).collect())
        })
    }
}

/// The TLS client configuration with the system roots and the bundled Mozilla roots.
pub fn default_tls_config() -> Result<Arc<ClientConfig>, rustls::Error> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let native = rustls_native_certs::load_native_certs();
    for certificate in native.certs {
        // A system root that rustls cannot parse is skipped; the bundled roots remain.
        let _ = roots.add(CertificateDer::from(certificate.to_vec()));
    }
    tls_config_with_roots(roots)
}

/// A configuration that trusts the given roots only. A test uses it with a self-signed root.
pub fn tls_config_with_roots(roots: RootCertStore) -> Result<Arc<ClientConfig>, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}
