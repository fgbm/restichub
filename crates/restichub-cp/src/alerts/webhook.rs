//! A small HTTP client for the webhook channels (FR-MN-04, ADR-0035).
//!
//! The client sends one signed `POST`. It resolves the host name, applies the address policy to
//! every answer, and connects to the approved address itself, so a second DNS answer cannot change
//! the target. It never follows a redirect and it reads the status line and the headers of the
//! answer only.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::pki_types::ServerName;

use super::settings::WebhookSettings;
use super::signature::{DELIVERY_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER, signature_header};
use super::ssrf::{Target, choose_address};
use super::transport::{BoxIo, Resolver, SendError};

/// Limit for connect, handshake and answer together.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// The most bytes of the answer head that the client reads.
const MAX_HEAD: usize = 16 * 1024;

/// What a send needs.
pub struct Request<'a> {
    pub target: &'a Target,
    pub body: &'a [u8],
    pub secret: &'a [u8],
    pub timestamp: i64,
    pub delivery_id: &'a str,
}

/// Sends the request and returns when the target answers with a 2xx status.
pub async fn send(
    policy: &WebhookSettings,
    resolver: &dyn Resolver,
    tls: &Arc<ClientConfig>,
    request: &Request<'_>,
) -> Result<(), SendError> {
    tokio::time::timeout(SEND_TIMEOUT, send_inner(policy, resolver, tls, request))
        .await
        .map_err(|_| SendError::Timeout)?
}

async fn send_inner(
    policy: &WebhookSettings,
    resolver: &dyn Resolver,
    tls: &Arc<ClientConfig>,
    request: &Request<'_>,
) -> Result<(), SendError> {
    let target = request.target;
    let addresses = match target.host.parse::<std::net::IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => resolver
            .resolve(&target.host, target.port)
            .await
            .map_err(|_| SendError::DnsFailed)?,
    };
    let address = choose_address(&addresses, target.port, policy)?;
    let tcp = TcpStream::connect(address)
        .await
        .map_err(|_| SendError::ConnectFailed)?;
    let mut stream: BoxIo = if target.tls {
        let name = ServerName::try_from(target.host.clone()).map_err(|_| SendError::TlsFailed)?;
        Box::new(
            TlsConnector::from(tls.clone())
                .connect(name, tcp)
                .await
                .map_err(|_| SendError::TlsFailed)?,
        )
    } else {
        Box::new(tcp)
    };

    let signature = signature_header(request.secret, request.timestamp, request.body);
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: ResticHub\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n{TIMESTAMP_HEADER}: {}\r\n{SIGNATURE_HEADER}: {signature}\r\n\
         {DELIVERY_HEADER}: {}\r\nConnection: close\r\n\r\n",
        target.request_uri,
        target.host_header(),
        request.body.len(),
        request.timestamp,
        request.delivery_id,
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(request.body).await?;
    stream.flush().await?;

    let status = read_status(&mut stream).await?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(SendError::HttpStatus(status))
    }
}

/// Reads until the end of the answer head and returns the status code.
async fn read_status(stream: &mut BoxIo) -> Result<u16, SendError> {
    let mut buffer = Vec::with_capacity(512);
    let mut chunk = [0_u8; 512];
    loop {
        if let Some(status) = parse_status(&buffer) {
            return status;
        }
        if buffer.len() >= MAX_HEAD {
            return Err(SendError::BadResponse);
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return parse_status(&buffer).unwrap_or(Err(SendError::BadResponse));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// `Some` once the status line is complete.
fn parse_status(buffer: &[u8]) -> Option<Result<u16, SendError>> {
    let end = buffer.windows(2).position(|pair| pair == b"\r\n")?;
    let line = String::from_utf8_lossy(&buffer[..end]);
    let mut parts = line.split(' ');
    let version = parts.next().unwrap_or_default();
    let code = parts.next().and_then(|code| code.parse::<u16>().ok());
    Some(match code {
        Some(code) if version.starts_with("HTTP/1.") && (100..600).contains(&code) => Ok(code),
        _ => Err(SendError::BadResponse),
    })
}
