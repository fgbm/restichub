//! The client settings of the stream connection, the same as the real agent uses, so the load
//! numbers match it.

use std::time::Duration;

use anyhow::Context as _;
use tonic::transport::{ClientTlsConfig, Endpoint};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(20);
const TCP_KEEPALIVE: Duration = Duration::from_secs(60);

pub(crate) fn endpoint(url: String, tls: ClientTlsConfig) -> anyhow::Result<Endpoint> {
    bounded(url, tls, CONNECT_TIMEOUT)
}

/// The TLS handshake has the same bound as the TCP connect: a control plane that cannot accept
/// leaves the connection in the listen backlog, where the TCP connect succeeds.
fn bounded(url: String, tls: ClientTlsConfig, timeout: Duration) -> anyhow::Result<Endpoint> {
    Ok(Endpoint::from_shared(url)
        .context("invalid server URL")?
        .tls_config(tls.timeout(timeout))?
        .connect_timeout(timeout)
        .http2_keep_alive_interval(KEEPALIVE_INTERVAL)
        .keep_alive_timeout(KEEPALIVE_TIMEOUT)
        .tcp_keepalive(Some(TCP_KEEPALIVE))
        .tcp_nodelay(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fr_id_08_a_server_that_never_answers_ends_the_connect_attempt() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let held = tokio::spawn(async move {
            let mut sockets = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                sockets.push(socket);
            }
        });
        let tls = ClientTlsConfig::new().domain_name("localhost");
        let endpoint = bounded(
            format!("https://{address}"),
            tls,
            Duration::from_millis(200),
        )
        .expect("endpoint");

        let attempt = tokio::time::timeout(Duration::from_secs(5), endpoint.connect()).await;

        assert!(
            matches!(attempt, Ok(Err(_))),
            "the attempt must fail on its own bound"
        );
        assert_eq!(
            endpoint.get_connect_timeout(),
            Some(Duration::from_millis(200))
        );
        held.abort();
    }
}
