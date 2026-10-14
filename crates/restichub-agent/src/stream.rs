//! The agent stream: the mTLS gRPC channel to the control plane (ADR-0026, ADR-0030, ADR-0033,
//! FR-ID-07, FR-ID-08, FR-PR-11).
//!
//! The agent opens one `AgentService.Connect` stream and keeps it open (FR-ID-07). The channel uses
//! mTLS: the client certificate is the stored leaf and its issuing CA, and the server is verified
//! only against the pinned installation root CA from the enrollment (ADR-0033); the system trust
//! store is never used. The endpoint must use `https` and must hold no userinfo.
//!
//! The agent answers each `Ping` with `Pong`, verifies each `SigningKeyUpdate` against the key ring
//! (ADR-0030) and answers with `KeyUpdateAck`. A dropped stream reconnects with exponential backoff
//! and full jitter, base 1 s and maximum 60 s (ADR-0026). The attempt grows unless the stream lived
//! at least one ping interval or answered a ping, so a server that closes at once is not hammered;
//! a stream that was working and then failed, such as on a control plane restart, resets it too.
//! An identity that the control plane refuses backs off at the maximum. A `SignedJob` is verified
//! and run off this task ([`crate::jobs`]), and a `CancelJob` stops the running job (FR-AG-10). The
//! agent starts no job on its own and holds no timer that starts one (ADR-0002).

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hyper::Uri;
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, ControlMessage, KeyUpdateAck, Ping, Pong, SigningKeyUpdate,
    agent_message, agent_service_client::AgentServiceClient, control_message,
};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use crate::identity::StoredIdentity;
use crate::jobs::Jobs;
use crate::keys::{Applied, KeyState};

/// The lower bound of the reconnect delay (ADR-0026).
pub const BACKOFF_BASE: Duration = Duration::from_secs(1);

/// The upper bound of the reconnect delay (ADR-0026).
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// The buffer of the messages that the agent sends to the control plane.
const OUTBOUND: usize = 16;

/// Reconnect delay with full jitter (ADR-0026, FR-ID-08).
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub base: Duration,
    pub max: Duration,
    /// Fixed jitter for a deterministic test; `None` draws from the entropy source.
    pub fraction: Option<f64>,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: BACKOFF_BASE,
            max: BACKOFF_MAX,
            fraction: None,
        }
    }
}

impl Backoff {
    /// The capped exponential value of one attempt (ADR-0026).
    fn cap(&self, attempt: u32) -> Duration {
        let shift = attempt.min(6);
        self.base.saturating_mul(1u32 << shift).min(self.max)
    }

    /// A wait of full jitter between zero and the capped exponential value (ADR-0026).
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        self.cap(attempt)
            .mul_f64(self.fraction.unwrap_or_else(random_fraction))
    }
}

/// A value in `[0, 1)` from the system entropy source. A failed read gives a fixed 0.5, so a
/// reconnect always waits a defined fraction of the cap (ADR-0026).
fn random_fraction() -> f64 {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return 0.5;
    }
    (u64::from_le_bytes(bytes) as f64) / (u64::MAX as f64)
}

/// The time budget of the connection and the heartbeat (FR-ID-07, FR-ID-08).
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// The interval between the heartbeat messages of the control plane (FR-ID-07).
    pub ping: Duration,
    /// The time budget of the TCP connect and the TLS handshake.
    pub connect: Duration,
    /// How often the client pings over HTTP/2.
    pub keepalive_interval: Duration,
    /// How long the client waits for the HTTP/2 ping answer.
    pub keepalive_timeout: Duration,
    /// Idle time before the first probe of a silent connection.
    pub tcp_keepalive: Duration,
}

impl Timeouts {
    /// The inbound idle limit: three ping intervals without a message closes the stream (FR-ID-07).
    #[must_use]
    pub fn idle(&self) -> Duration {
        self.ping.saturating_mul(3)
    }
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            ping: Duration::from_secs(30),
            connect: Duration::from_secs(10),
            keepalive_interval: Duration::from_secs(30),
            keepalive_timeout: Duration::from_secs(20),
            tcp_keepalive: Duration::from_secs(60),
        }
    }
}

/// The reconnect and timeout parameters of the connect loop.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamOptions {
    pub backoff: Backoff,
    pub timeouts: Timeouts,
}

/// Why one connection attempt ended (FR-ID-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disconnect {
    /// The stream ended. `healthy` is true when it lived at least one ping interval or answered a
    /// ping, so the reconnect attempt resets (FR-ID-08).
    Closed { healthy: bool },
    /// The control plane refused the identity: the agent is revoked or unknown. Back off at the
    /// maximum (FR-ID-04).
    Refused,
}

/// Runs the connect loop with the operating system shutdown signal (FR-AG-01, FR-ID-08).
///
/// The systemd unit sends SIGTERM and the Windows service sends a stop control; both reach the
/// blocking `shutdown` channel, so the loop stops gracefully.
///
/// # Errors
///
/// Returns an error when the signal handler cannot be installed.
pub async fn serve(
    identity: &StoredIdentity,
    dir: &Path,
    keys: &mut KeyState,
    restic_version: &str,
    jobs: &Jobs,
    shutdown: Option<Receiver<()>>,
) -> Result<()> {
    let (stop_tx, mut stop_rx) = watch::channel(false);
    spawn_shutdown(shutdown, stop_tx)?;
    run(
        identity,
        dir,
        keys,
        restic_version,
        jobs,
        StreamOptions::default(),
        &mut stop_rx,
    )
    .await
}

/// Bridges the blocking shutdown channel to the async stop signal.
fn spawn_shutdown(shutdown: Option<Receiver<()>>, stop: watch::Sender<bool>) -> Result<()> {
    let receiver = match shutdown {
        Some(receiver) => receiver,
        None => {
            let (sender, receiver) = std::sync::mpsc::channel();
            ctrlc::set_handler(move || {
                let _ = sender.send(());
            })
            .context("cannot install the signal handler")?;
            receiver
        }
    };
    std::thread::spawn(move || {
        let _ = receiver.recv();
        let _ = stop.send(true);
    });
    Ok(())
}

/// Keeps the agent connected until `stop` becomes true (FR-ID-07, FR-ID-08).
///
/// # Errors
///
/// Returns an error only when the key chain cannot be read. A failed connection is retried.
pub async fn run(
    identity: &StoredIdentity,
    dir: &Path,
    keys: &mut KeyState,
    restic_version: &str,
    jobs: &Jobs,
    options: StreamOptions,
    stop: &mut watch::Receiver<bool>,
) -> Result<()> {
    let mut attempt = 0_u32;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let (refused, healthy) = match connect_once(
            identity,
            dir,
            keys,
            restic_version,
            jobs,
            options.timeouts,
            stop,
        )
        .await
        {
            Ok(Disconnect::Refused) => {
                tracing::warn!(
                    agent_id = %identity.agent_id,
                    "the control plane refused the agent identity; check that the agent is enrolled and not revoked"
                );
                (true, false)
            }
            Ok(Disconnect::Closed { healthy }) => (false, healthy),
            Err(error) => {
                tracing::warn!(agent_id = %identity.agent_id, %error, "the agent stream stopped");
                (false, false)
            }
        };
        if *stop.borrow() {
            return Ok(());
        }
        let delay = if refused {
            options.backoff.max
        } else {
            options.backoff.delay(attempt)
        };
        attempt = if refused {
            attempt.saturating_add(1)
        } else {
            next_attempt(attempt, healthy)
        };
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

/// How the reconnect counter moves after one stream ends. A healthy stream resets it; a stream
/// that closed at once or failed grows it (FR-ID-08).
fn next_attempt(attempt: u32, healthy: bool) -> u32 {
    if healthy {
        0
    } else {
        attempt.saturating_add(1)
    }
}

/// Opens one stream and serves the control messages until it ends (FR-ID-07, FR-PR-11).
///
/// A transport failure after the stream opened ends the attempt with [`Disconnect::Closed`], so a
/// healthy stream that a control plane restart dropped resets the reconnect attempt (FR-ID-08).
///
/// # Errors
///
/// Returns an error only when the channel cannot be built or the first message cannot be sent. A
/// refusal of the identity is `Ok(Disconnect::Refused)`, not an error.
pub async fn connect_once(
    identity: &StoredIdentity,
    dir: &Path,
    keys: &mut KeyState,
    restic_version: &str,
    jobs: &Jobs,
    timeouts: Timeouts,
    stop: &mut watch::Receiver<bool>,
) -> Result<Disconnect> {
    let channel = channel(identity, timeouts).await?;
    let mut client = AgentServiceClient::new(channel)
        .max_decoding_message_size(restichub_proto::MAX_MESSAGE_BYTES)
        .max_encoding_message_size(restichub_proto::MAX_MESSAGE_BYTES);
    let (tx, rx) = mpsc::channel::<AgentMessage>(OUTBOUND);
    let response = match client.connect(ReceiverStream::new(rx)).await {
        Ok(response) => response,
        Err(status) if refused(status.code()) => return Ok(Disconnect::Refused),
        Err(status) => return Err(anyhow::anyhow!("the stream did not open: {status}")),
    };
    let mut inbound = response.into_inner();
    send_hello(&tx, restic_version).await?;
    // The results go through this stream now, and a result that a dropped stream lost reaches the
    // control plane (ADR-0020).
    jobs.attach(&tx);
    jobs.send_stored_results().await;
    let started = tokio::time::Instant::now();
    let mut pinged = false;
    loop {
        let received = tokio::select! {
            received = tokio::time::timeout(timeouts.idle(), inbound.message()) => received,
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    return Ok(Disconnect::Closed { healthy: pinged || started.elapsed() >= timeouts.ping });
                }
                continue;
            }
        };
        let message = match received {
            Ok(Ok(message)) => message,
            // An error after the stream opened is the transport failure of a stream that was
            // working: a control plane restart or a deploy. A stream that lived one ping interval or
            // answered a ping is healthy, so the reconnect attempt resets (FR-ID-08).
            Ok(Err(status)) => {
                tracing::warn!(%status, "the agent stream failed");
                return Ok(Disconnect::Closed {
                    healthy: pinged || started.elapsed() >= timeouts.ping,
                });
            }
            // No message for three ping intervals: the control plane is silent, so reconnect
            // (FR-ID-07).
            Err(_) => {
                tracing::warn!("the control plane sent no message for three ping intervals");
                return Ok(Disconnect::Closed { healthy: false });
            }
        };
        let Some(message) = message else {
            return Ok(Disconnect::Closed {
                healthy: pinged || started.elapsed() >= timeouts.ping,
            });
        };
        if matches!(message.payload, Some(control_message::Payload::Ping(_))) {
            pinged = true;
        }
        if let Err(error) = handle_control(identity, keys, dir, jobs, &tx, message).await {
            tracing::warn!(%error, "the agent stream stopped");
            return Ok(Disconnect::Closed {
                healthy: pinged || started.elapsed() >= timeouts.ping,
            });
        }
    }
}

/// Matches the status codes of a revoked or an unknown agent (FR-ID-04).
fn refused(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unauthenticated | tonic::Code::PermissionDenied
    )
}

/// Serves one control message from the control plane (FR-ID-07, FR-PR-03, FR-PR-11, FR-AG-10).
///
/// The identity comes from the certificate, never from a message (FR-PR-08). A signed job is
/// verified with the key ring and run off this task; a cancellation stops the running job.
async fn handle_control(
    identity: &StoredIdentity,
    keys: &mut KeyState,
    dir: &Path,
    jobs: &Jobs,
    tx: &mpsc::Sender<AgentMessage>,
    message: ControlMessage,
) -> Result<()> {
    match message.payload {
        Some(control_message::Payload::ServerHello(hello)) => {
            tracing::info!(
                major = hello.protocol_version_major,
                minor = hello.protocol_version_minor,
                "the control plane greeted the agent"
            );
        }
        Some(control_message::Payload::Ping(ping)) => {
            let _ = tx.send(pong(&ping)).await;
        }
        Some(control_message::Payload::SignedJob(signed)) => {
            jobs.handle(signed, keys.ring(), &identity.agent_id, &identity.tenant_id)
                .await;
        }
        Some(control_message::Payload::CancelJob(cancel)) => {
            jobs.cancel(&cancel.job_id);
        }
        Some(control_message::Payload::SigningKeyUpdate(update)) => match keys.apply(&update) {
            Ok(Applied::Changed) => {
                // The chain is durable before the acknowledgement (ADR-0030). A write error keeps
                // the stream and the in-memory ring, but the agent does not acknowledge the update,
                // so the control plane keeps the old key until a later attempt persists the chain.
                match keys.persist(dir) {
                    Ok(()) => {
                        let _ = tx.send(ack(&update)).await;
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "cannot write the job signing key chain; the agent does not acknowledge the update"
                        );
                    }
                }
            }
            Ok(Applied::Unchanged) => {
                // The ring already holds the key and the revocations; the chain file is unchanged.
                let _ = tx.send(ack(&update)).await;
            }
            Ok(Applied::Skipped) => {
                tracing::debug!(
                    signer = %update.signer_key_id,
                    "a key update step of an earlier rotation arrived; the agent already applied it"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "the signing key update was refused");
            }
        },
        Some(control_message::Payload::CertificateIssued(_)) => {
            // Certificate rotation needs the enrollment flow of another epic.
            tracing::info!("a certificate rotation arrived; the agent ignores it");
        }
        None => {}
    }
    Ok(())
}

/// Answers a heartbeat with the echoed nonce (FR-ID-07).
#[must_use]
pub fn pong(ping: &Ping) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::Pong(Pong { nonce: ping.nonce })),
    }
}

/// The acknowledgement of one accepted key update (FR-PR-11).
fn ack(update: &SigningKeyUpdate) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::KeyUpdateAck(KeyUpdateAck {
            key_id: update.signer_key_id.clone(),
        })),
    }
}

/// Builds the mTLS connection to the listener (ADR-0026, ADR-0033).
///
/// The client certificate is the stored leaf and its issuing CA. The server is verified only against
/// the pinned installation root CA from the enrollment, never the system trust store.
async fn channel(identity: &StoredIdentity, timeouts: Timeouts) -> Result<Channel> {
    let endpoint = identity
        .grpc_endpoint
        .as_deref()
        .context("the identity holds no gRPC endpoint")?;
    let uri: Uri = endpoint
        .parse()
        .context("the gRPC endpoint is not a valid URL")?;
    let host = endpoint_host(&uri)?;
    let root = identity
        .ca_chain_der
        .last()
        .context("the identity holds no root CA")?;
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(crate::pem::encode(
            "CERTIFICATE",
            root,
        )))
        .identity(Identity::from_pem(
            client_chain_pem(identity),
            identity.key_pem.clone(),
        ))
        .domain_name(host);
    let channel = Endpoint::from_shared(endpoint.to_owned())
        .context("the gRPC endpoint is not a valid URL")?
        .tls_config(tls)
        .context("cannot build the client TLS configuration")?
        .connect_timeout(timeouts.connect)
        .http2_keep_alive_interval(timeouts.keepalive_interval)
        .keep_alive_timeout(timeouts.keepalive_timeout)
        .tcp_keepalive(Some(timeouts.tcp_keepalive))
        .tcp_nodelay(true)
        .connect()
        .await
        .context("cannot connect to the control plane")?;
    Ok(channel)
}

/// The stored leaf and its issuers in PEM, without the pinned root.
fn client_chain_pem(identity: &StoredIdentity) -> String {
    let mut pem = crate::pem::encode("CERTIFICATE", &identity.certificate_der);
    let issuing = identity.ca_chain_der.len().saturating_sub(1);
    for der in &identity.ca_chain_der[..issuing] {
        pem.push_str(&crate::pem::encode("CERTIFICATE", der));
    }
    pem
}

/// The server name of an `https` URL. A non-`https` scheme and a userinfo are refused (ADR-0033).
///
/// # Errors
///
/// Returns an error when the scheme is not `https`, when the authority holds a userinfo, or when
/// the host is absent.
fn endpoint_host(uri: &Uri) -> Result<String> {
    if uri.scheme_str() != Some("https") {
        bail!("the gRPC endpoint must use the `https` scheme");
    }
    let authority = uri.authority().context("the gRPC endpoint holds no host")?;
    if authority.as_str().contains('@') {
        bail!("the gRPC endpoint must not hold a userinfo");
    }
    let host = authority.host();
    let host = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() {
        bail!("the gRPC endpoint holds no host");
    }
    Ok(host.to_owned())
}

/// Sends the first message of the agent: the identity and the capabilities (FR-PR-09).
async fn send_hello(tx: &mpsc::Sender<AgentMessage>, restic_version: &str) -> Result<()> {
    let hello = AgentMessage {
        payload: Some(agent_message::Payload::AgentHello(AgentHello {
            protocol_version_major: restichub_proto::PROTOCOL_VERSION_MAJOR,
            protocol_version_minor: restichub_proto::PROTOCOL_VERSION_MINOR,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            restic_version: restic_version.to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            capabilities: restichub_proto::AgentCapability::ALL
                .iter()
                .map(|capability| capability.as_str().to_owned())
                .collect(),
            ..AgentHello::default()
        })),
    };
    tx.send(hello).await.context("cannot send AgentHello")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{Backoff, endpoint_host, next_attempt, pong, refused, send_hello};
    use restichub_proto::agent::v1::{Ping, agent_message};

    #[test]
    fn m2_a_stream_that_closes_at_once_grows_the_delay() {
        let backoff = Backoff::default();
        let mut attempt = 0;
        for _ in 0..5 {
            // A stream that closed at once is not healthy, so the attempt grows.
            attempt = next_attempt(attempt, false);
        }
        assert_eq!(attempt, 5);
        assert!(backoff.cap(attempt) > backoff.cap(0));
        // Reset the attempt when the stream was healthy.
        assert_eq!(next_attempt(attempt, true), 0);
    }

    #[test]
    fn the_backoff_cap_grows_and_stays_bounded() {
        let backoff = Backoff::default();
        for attempt in 0..20 {
            assert!(backoff.cap(attempt) <= backoff.max, "attempt {attempt}");
        }
        assert!(backoff.delay(0) <= backoff.base);
    }

    #[test]
    fn m1_the_endpoint_must_be_https_without_a_userinfo() {
        let host = endpoint_host(&"https://agents.example:8443".parse().expect("uri"))
            .expect("a valid endpoint");
        assert_eq!(host, "agents.example");
        let host = endpoint_host(&"https://[2001:db8::1]:8443".parse().expect("uri"))
            .expect("an IPv6 endpoint");
        assert_eq!(host, "2001:db8::1");
        assert!(endpoint_host(&"http://agents.example".parse().expect("uri")).is_err());
        assert!(endpoint_host(&"https://user:pass@agents.example".parse().expect("uri")).is_err());
    }

    #[test]
    fn ac_id_07_a_ping_is_answered_with_the_nonce() {
        let message = pong(&Ping { nonce: 42 });
        let Some(agent_message::Payload::Pong(pong)) = message.payload else {
            panic!("the answer must be a Pong");
        };
        assert_eq!(pong.nonce, 42);
    }

    #[test]
    fn only_a_refused_identity_backs_off_at_the_maximum() {
        assert!(refused(tonic::Code::Unauthenticated));
        assert!(refused(tonic::Code::PermissionDenied));
        assert!(!refused(tonic::Code::Internal));
        assert!(!refused(tonic::Code::Unavailable));
    }

    #[tokio::test]
    async fn fr_pr_09_the_hello_announces_the_forget_all_tags_capability() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        send_hello(&tx, "0.19.0").await.expect("the hello is sent");
        let message = rx.recv().await.expect("a message");
        let Some(agent_message::Payload::AgentHello(hello)) = message.payload else {
            panic!("the first message must be an AgentHello");
        };
        assert!(restichub_proto::AgentCapability::ForgetAllTags.is_listed(&hello.capabilities));
    }
}
