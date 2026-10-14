//! The agent stream: the mTLS gRPC channel to the control plane (ADR-0026, ADR-0030, ADR-0033,
//! FR-ID-07, FR-ID-08, FR-PR-11).
//!
//! The agent opens one `AgentService.Connect` stream and keeps it open (FR-ID-07). The channel uses
//! mTLS: the client certificate is the stored leaf and its issuing CA, and the server is verified
//! only against the pinned installation root CA from the enrollment (ADR-0033); the system trust
//! store is never used. The listener offers TLS 1.3 only, so the connection negotiates TLS 1.3.
//!
//! The agent answers each `Ping` with `Pong`, verifies each `SigningKeyUpdate` against the key ring
//! (ADR-0030) and answers with `KeyUpdateAck`. A dropped stream reconnects with exponential backoff
//! and full jitter, base 1 s and maximum 60 s (ADR-0026). An identity that the control plane refuses
//! backs off at the maximum. The agent starts no job and holds no timer that starts one (ADR-0002);
//! a job or a cancellation message is logged and ignored until epic #7.

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use prost::Message as _;
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, ControlMessage, KeyUpdate, KeyUpdateAck, Ping, Pong,
    SigningKeyUpdate, agent_message, agent_service_client::AgentServiceClient, control_message,
};
use restichub_proto::signing::{KeyRing, KeyUpdateError};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use crate::identity::StoredIdentity;

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
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: BACKOFF_BASE,
            max: BACKOFF_MAX,
        }
    }
}

impl Backoff {
    /// A wait of full jitter between zero and the capped exponential value (ADR-0026).
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        let shift = attempt.min(6);
        let cap = self.base.saturating_mul(1u32 << shift).min(self.max);
        cap.mul_f64(random_fraction())
    }
}

fn random_fraction() -> f64 {
    let mut bytes = [0_u8; 8];
    let _ = getrandom::fill(&mut bytes);
    (u64::from_le_bytes(bytes) as f64) / (u64::MAX as f64)
}

/// Why one connection attempt ended (FR-ID-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disconnect {
    /// The stream ended or the connection failed. Reconnect with the exponential backoff.
    Closed,
    /// The control plane refused the identity: the agent is revoked or unknown. Back off at the
    /// maximum (FR-ID-04).
    Refused,
}

/// Builds the trust set of a stored identity (ADR-0026, ADR-0030).
///
/// The enrollment set gives the current key and the next key; the persisted chain then applies each
/// later rotation, so a job that the current key signed verifies after a restart.
///
/// # Errors
///
/// Returns an error when the enrollment set is not valid or a stored update does not verify.
pub fn key_ring(identity: &StoredIdentity) -> Result<KeyRing> {
    let mut ring = crate::verify::verify_key_set(
        &identity.signing_keys.keys,
        &identity.signing_keys.signature,
    )?;
    for entry in &identity.key_updates {
        let bytes = STANDARD
            .decode(entry)
            .context("a stored key update is not base64")?;
        let update = SigningKeyUpdate::decode(bytes.as_slice())
            .context("a stored key update does not decode")?;
        ring.apply_key_update(&update)
            .map_err(|error| anyhow::anyhow!("a stored key update is not valid: {error:?}"))?;
    }
    Ok(ring)
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
    identity: &mut StoredIdentity,
    dir: &Path,
    ring: &mut KeyRing,
    restic_version: &str,
    shutdown: Option<Receiver<()>>,
) -> Result<()> {
    let (stop_tx, mut stop_rx) = watch::channel(false);
    spawn_shutdown(shutdown, stop_tx)?;
    run(
        identity,
        dir,
        ring,
        restic_version,
        Backoff::default(),
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
    identity: &mut StoredIdentity,
    dir: &Path,
    ring: &mut KeyRing,
    restic_version: &str,
    backoff: Backoff,
    stop: &mut watch::Receiver<bool>,
) -> Result<()> {
    let mut attempt = 0_u32;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let refused = match connect_once(identity, dir, ring, restic_version, stop).await {
            Ok(Disconnect::Refused) => {
                tracing::warn!(
                    agent_id = %identity.agent_id,
                    "the control plane refused the agent identity; check that the agent is enrolled and not revoked"
                );
                true
            }
            Ok(Disconnect::Closed) => {
                attempt = 0;
                false
            }
            Err(error) => {
                tracing::warn!(agent_id = %identity.agent_id, %error, "the agent stream stopped");
                false
            }
        };
        if *stop.borrow() {
            return Ok(());
        }
        let delay = if refused {
            backoff.max
        } else {
            backoff.delay(attempt)
        };
        attempt = attempt.saturating_add(1);
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

/// Opens one stream and serves the control messages until it ends (FR-ID-07, FR-PR-11).
///
/// # Errors
///
/// Returns an error when the channel cannot be built or a message cannot be sent. A refusal of the
/// identity is `Ok(Disconnect::Refused)`, not an error.
pub async fn connect_once(
    identity: &mut StoredIdentity,
    dir: &Path,
    ring: &mut KeyRing,
    restic_version: &str,
    stop: &mut watch::Receiver<bool>,
) -> Result<Disconnect> {
    let channel = channel(identity).await?;
    let mut client = AgentServiceClient::new(channel);
    let (tx, rx) = mpsc::channel::<AgentMessage>(OUTBOUND);
    let response = match client.connect(ReceiverStream::new(rx)).await {
        Ok(response) => response,
        Err(status) if refused(status.code()) => return Ok(Disconnect::Refused),
        Err(status) => return Err(anyhow::anyhow!("the stream did not open: {status}")),
    };
    let mut inbound = response.into_inner();
    send_hello(&tx, restic_version).await?;
    loop {
        let message = tokio::select! {
            message = inbound.message() => message,
            result = stop.changed() => {
                if result.is_err() || *stop.borrow() {
                    return Ok(Disconnect::Closed);
                }
                continue;
            }
        };
        let Some(message) = message.context("the stream failed")? else {
            return Ok(Disconnect::Closed);
        };
        handle_control(identity, dir, ring, &tx, message).await?;
    }
}

/// Matches the status codes of a revoked or an unknown agent (FR-ID-04).
fn refused(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unauthenticated | tonic::Code::PermissionDenied
    )
}

/// Serves one control message from the control plane (FR-ID-07, FR-PR-11).
///
/// The identity comes from the certificate, never from a message (FR-PR-08). Job delivery,
/// cancellation and certificate rotation belong to later epics; the agent logs them and continues.
async fn handle_control(
    identity: &mut StoredIdentity,
    dir: &Path,
    ring: &mut KeyRing,
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
        Some(control_message::Payload::SigningKeyUpdate(update)) => {
            match accept_update(ring, &update) {
                Ok(key_id) => {
                    remember_update(identity, &update, &key_id)?;
                    // The new set is durable before the acknowledgement, so a restart keeps it
                    // (ADR-0030).
                    identity.save(dir)?;
                    let ack = AgentMessage {
                        payload: Some(agent_message::Payload::KeyUpdateAck(KeyUpdateAck {
                            key_id: update.signer_key_id.clone(),
                        })),
                    };
                    let _ = tx.send(ack).await;
                }
                Err(error) => {
                    tracing::warn!(?error, "the signing key update was refused");
                }
            }
        }
        Some(
            control_message::Payload::SignedJob(_)
            | control_message::Payload::CancelJob(_)
            | control_message::Payload::CertificateIssued(_),
        ) => {
            tracing::info!("a control message of a later epic arrived; the agent ignores it");
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

/// Verifies one signed update and applies it to the ring, and returns the key it introduces
/// (FR-PR-11, ADR-0030).
///
/// # Errors
///
/// Returns the [`KeyUpdateError`] of the ring for a bad signature, an unknown or revoked signer, a
/// key identifier conflict or a malformed update.
pub fn accept_update(
    ring: &mut KeyRing,
    update: &SigningKeyUpdate,
) -> Result<String, KeyUpdateError> {
    ring.apply_key_update(update)?;
    let inner =
        KeyUpdate::decode(update.key_update.as_slice()).map_err(|_| KeyUpdateError::Malformed)?;
    Ok(inner.key_id)
}

/// Records an accepted update in the persisted chain, one entry for each introduced key (ADR-0030).
///
/// The control plane re-sends the current set on every connect, so an entry that introduces the same
/// key is replaced instead of appended.
fn remember_update(
    identity: &mut StoredIdentity,
    update: &SigningKeyUpdate,
    key_id: &str,
) -> Result<()> {
    let encoded = STANDARD.encode(update.encode_to_vec());
    if let Some(index) = identity
        .key_updates
        .iter()
        .position(|entry| stored_key_id(entry).as_deref() == Some(key_id))
    {
        identity.key_updates[index] = encoded;
        return Ok(());
    }
    identity.key_updates.push(encoded);
    Ok(())
}

/// The key that a stored envelope introduces, or `None` when it does not decode.
fn stored_key_id(entry: &str) -> Option<String> {
    let bytes = STANDARD.decode(entry).ok()?;
    let update = SigningKeyUpdate::decode(bytes.as_slice()).ok()?;
    let inner = KeyUpdate::decode(update.key_update.as_slice()).ok()?;
    Some(inner.key_id)
}

/// Builds the mTLS connection to the listener (ADR-0026, ADR-0033).
///
/// The client certificate is the stored leaf and its issuing CA. The server is verified only against
/// the pinned installation root CA from the enrollment, never the system trust store.
async fn channel(identity: &StoredIdentity) -> Result<Channel> {
    let endpoint = identity
        .grpc_endpoint
        .as_deref()
        .context("the identity holds no gRPC endpoint")?;
    let host = endpoint_host(endpoint).context("the gRPC endpoint holds no host")?;
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

/// The host of a URL such as `https://agents.example:8443` (ADR-0026).
fn endpoint_host(endpoint: &str) -> Option<String> {
    let rest = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(authority);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (!host.is_empty()).then(|| host.to_owned())
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
            ..AgentHello::default()
        })),
    };
    tx.send(hello).await.context("cannot send AgentHello")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{Backoff, endpoint_host, pong, refused};
    use restichub_proto::agent::v1::{Ping, agent_message};

    #[test]
    fn the_backoff_grows_and_stays_bounded() {
        let backoff = Backoff::default();
        for attempt in 0..20 {
            let delay = backoff.delay(attempt);
            assert!(delay <= backoff.max, "attempt {attempt} exceeds the cap");
        }
        // Full jitter gives a value below the cap of the attempt.
        assert!(backoff.delay(0) <= backoff.base);
    }

    #[test]
    fn the_endpoint_host_ignores_the_scheme_and_the_port() {
        assert_eq!(
            endpoint_host("https://agents.example:8443").as_deref(),
            Some("agents.example")
        );
        assert_eq!(
            endpoint_host("https://[2001:db8::1]:8443").as_deref(),
            Some("2001:db8::1")
        );
        assert_eq!(endpoint_host("").as_deref(), None);
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
}
