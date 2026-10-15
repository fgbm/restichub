//! Enrollment of a fleet of emulated agents at a deployed control plane (bench S2).
//!
//! The fleet gets one enrollment token for each agent, either from a token file or from the
//! public API, and enrolls every agent over HTTPS with the installation root CA (ADR-0026). The
//! agents then use the issued certificate for the stream, so the control plane sees real agents.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::VerifyingKey;
use restichub_proto::signing::KeyRing;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::enroll::{
    EnrollFacts, EnrollTls, EnrolledIdentity, create_enrollment_token, enroll_tls,
};
use crate::identity::AgentIdentity;

/// Where the enrollment tokens come from.
#[derive(Debug, Clone)]
pub enum TokenSource {
    /// A text file with one token on each line.
    File(PathBuf),
    /// The enrollment token route of the API, called with an API token.
    Api { url: String, api_token: String },
}

/// What the fleet needs to enroll its agents.
#[derive(Debug, Clone)]
pub struct FleetConfig {
    pub agents: usize,
    /// Base URL of the enrollment listener.
    pub enroll_url: String,
    pub tokens: TokenSource,
    /// Root CA of the installation, in PEM.
    pub ca_pem: String,
    pub server_name: String,
    /// How many enrollments run at the same time.
    pub concurrency: usize,
    pub hostname_prefix: String,
}

/// The identities and the trust material that the enrollment returns.
pub struct Fleet {
    pub identities: Vec<AgentIdentity>,
    pub key_ring: KeyRing,
    pub tenant_id: String,
    /// The gRPC endpoint that the control plane announced, when it announced one.
    pub grpc_endpoint: Option<String>,
}

const ATTEMPTS: usize = 4;
const TOKEN_TTL_SECONDS: i64 = 3600;

/// Enrolls `config.agents` agents and returns their identities.
///
/// # Errors
///
/// Returns an error when a token is missing, an enrollment keeps failing, or the control plane
/// returns an unusable key set.
pub async fn enroll_fleet(config: &FleetConfig) -> anyhow::Result<Fleet> {
    let tls = EnrollTls {
        ca_der: pem_to_der(&config.ca_pem)?,
        server_name: config.server_name.clone(),
    };
    let tokens = collect_tokens(config, &tls).await?;
    let semaphore = Arc::new(Semaphore::new(config.concurrency.max(1)));
    let mut tasks = JoinSet::new();
    for (index, token) in tokens.into_iter().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        let url = config.enroll_url.clone();
        let tls = tls.clone();
        let facts = EnrollFacts {
            hostname: format!("{}-{index:05}", config.hostname_prefix),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            restic_version: "0.18.0".to_owned(),
        };
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .context("the enrollment limiter closed")?;
            let identity = enroll_with_retry(&url, &token, &facts, &tls).await?;
            anyhow::Ok((index, identity))
        });
    }
    let mut enrolled: Vec<Option<EnrolledIdentity>> = (0..config.agents).map(|_| None).collect();
    while let Some(joined) = tasks.join_next().await {
        let (index, identity) = joined.context("an enrollment task panicked")??;
        enrolled[index] = Some(identity);
    }
    let enrolled = enrolled
        .into_iter()
        .map(|entry| entry.ok_or_else(|| anyhow!("an enrollment is missing")))
        .collect::<anyhow::Result<Vec<_>>>()?;
    fleet_from(enrolled)
}

async fn collect_tokens(config: &FleetConfig, tls: &EnrollTls) -> anyhow::Result<Vec<String>> {
    match &config.tokens {
        TokenSource::File(path) => read_tokens(path, config.agents),
        TokenSource::Api { url, api_token } => {
            let api_tls = url.starts_with("https://").then_some(tls);
            let mut tokens = Vec::with_capacity(config.agents);
            for _ in 0..config.agents {
                let token = create_enrollment_token(url, api_token, TOKEN_TTL_SECONDS, api_tls)
                    .await
                    .context("cannot create an enrollment token")?;
                tokens.push(token);
            }
            Ok(tokens)
        }
    }
}

fn read_tokens(path: &Path, agents: usize) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the token file {}", path.display()))?;
    let tokens: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if tokens.len() < agents {
        bail!(
            "the token file holds {} tokens, but {agents} agents need one each",
            tokens.len()
        );
    }
    Ok(tokens.into_iter().take(agents).collect())
}

async fn enroll_with_retry(
    url: &str,
    token: &str,
    facts: &EnrollFacts,
    tls: &EnrollTls,
) -> anyhow::Result<EnrolledIdentity> {
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match enroll_tls(url, token, facts, tls).await {
            Ok(identity) => return Ok(identity),
            Err(error) => {
                last = Some(error);
                tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("no enrollment attempt ran")))
        .with_context(|| format!("the enrollment of {} failed", facts.hostname))
}

fn fleet_from(enrolled: Vec<EnrolledIdentity>) -> anyhow::Result<Fleet> {
    let first = enrolled
        .first()
        .ok_or_else(|| anyhow!("the fleet has no agent"))?;
    let mut key_ring = KeyRing::new();
    for key in &first.signing_keys {
        let bytes: [u8; 32] = STANDARD
            .decode(&key.public_key)
            .context("a job signing key is not base64")?
            .try_into()
            .map_err(|bytes: Vec<u8>| {
                anyhow!("a job signing key holds {} bytes, not 32", bytes.len())
            })?;
        let verifying = VerifyingKey::from_bytes(&bytes)
            .map_err(|error| anyhow!("invalid job signing key: {error}"))?;
        key_ring
            .insert(key.key_id.clone(), verifying)
            .map_err(|error| anyhow!("cannot insert the job signing key: {error:?}"))?;
    }
    let tenant_id = first.tenant_id.clone();
    let grpc_endpoint = first.grpc_endpoint.clone();
    let identities = enrolled
        .into_iter()
        .map(|entry| AgentIdentity {
            agent_id: entry.agent_id,
            cert_pem: der_to_pem(&entry.certificate_der),
            key_pem: entry.key_pem,
        })
        .collect();
    Ok(Fleet {
        identities,
        key_ring,
        tenant_id,
        grpc_endpoint,
    })
}

/// Reads the first certificate of a PEM text as DER.
fn pem_to_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    let body: String = pem
        .lines()
        .skip_while(|line| !line.starts_with("-----BEGIN CERTIFICATE"))
        .skip(1)
        .take_while(|line| !line.starts_with("-----END"))
        .collect();
    if body.is_empty() {
        bail!("the CA file holds no certificate");
    }
    STANDARD
        .decode(body.trim())
        .context("the CA certificate is not base64")
}

fn der_to_pem(der: &[u8]) -> String {
    let encoded = STANDARD.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_survives_the_pem_round_trip() {
        let der: Vec<u8> = (0..=200).collect();

        assert_eq!(pem_to_der(&der_to_pem(&der)).expect("der"), der);
    }

    #[test]
    fn a_short_token_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("rh-fleet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("tokens.txt");
        std::fs::write(&file, "one\ntwo\n").expect("write");

        assert!(read_tokens(&file, 3).is_err());
        assert_eq!(read_tokens(&file, 2).expect("tokens").len(), 2);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
