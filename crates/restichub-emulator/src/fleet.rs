//! Enrollment of a fleet of emulated agents at a deployed control plane (bench S2).
//!
//! The fleet gets one enrollment token for each agent, either from a token file or from the
//! public API, and enrolls every agent over HTTPS with the installation root CA (ADR-0026). The
//! agents then use the issued certificate for the stream, so the control plane sees real agents.
//!
//! The control plane limits the enrollments of one source address to 10 each minute (FR-ID-05),
//! and all emulated agents share one address. The fleet therefore paces the enrollments, and it
//! keeps each identity in a store directory as soon as the enrollment ends. A later run reads the
//! store and enrolls only the missing agents, so the slow enrollment of 500 agents runs once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::VerifyingKey;
use restichub_proto::signing::KeyRing;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

use crate::enroll::{
    EnrollFacts, EnrollTls, EnrolledIdentity, create_enrollment_token, enroll_tls,
};
use crate::identity::AgentIdentity;
use crate::lock;

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
    /// The most enrollments that start in one minute. Zero turns the pacing off.
    pub per_minute: u32,
    pub hostname_prefix: String,
    /// Directory that keeps the enrolled identities between runs.
    pub store: Option<PathBuf>,
}

/// The identities and the trust material that the enrollment returns.
pub struct Fleet {
    pub identities: Vec<AgentIdentity>,
    pub key_ring: KeyRing,
    pub tenant_id: String,
    /// The gRPC endpoint that the control plane announced, when it announced one.
    pub grpc_endpoint: Option<String>,
}

/// What every agent of the installation shares. The store keeps it in `fleet.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Shared {
    tenant_id: String,
    grpc_endpoint: Option<String>,
    signing_keys: Vec<SharedKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SharedKey {
    key_id: String,
    /// Base64 of the 32 Ed25519 bytes.
    public_key: String,
}

const ATTEMPTS: usize = 6;
const TOKEN_TTL_SECONDS: i64 = 3600;
const SHARED_FILE: &str = "fleet.json";

/// Enrolls the agents that the store does not hold yet and returns all `config.agents` identities.
///
/// # Errors
///
/// Returns an error when a token is missing, an enrollment keeps failing, a store file cannot be
/// written, or the control plane returns an unusable key set.
pub async fn enroll_fleet(config: &FleetConfig) -> anyhow::Result<Fleet> {
    let mut identities = match &config.store {
        Some(dir) => read_store(dir)?,
        None => Vec::new(),
    };
    identities.truncate(config.agents);
    let mut shared = match &config.store {
        Some(dir) => read_shared(dir)?,
        None => None,
    };
    let missing = config.agents - identities.len();
    if missing > 0 {
        let tls = EnrollTls {
            ca_der: pem_to_der(&config.ca_pem)?,
            server_name: config.server_name.clone(),
        };
        let tokens = match &config.tokens {
            TokenSource::File(path) => read_tokens(path, missing)?,
            TokenSource::Api { .. } => Vec::new(),
        };
        let first = identities.len();
        let store = Arc::new(Store::new(config.store.clone()));
        let enrolled = enroll_missing(config, &tls, tokens, first, missing, &store).await?;
        for entry in enrolled {
            if shared.is_none() {
                shared = Some(shared_from(&entry));
                store.write_shared(shared.as_ref())?;
            }
            let cert_pem = chain_pem(&entry);
            identities.push(AgentIdentity {
                agent_id: entry.agent_id,
                cert_pem,
                key_pem: entry.key_pem,
            });
        }
    }
    let shared = shared.ok_or_else(|| anyhow!("the fleet has no agent"))?;
    Ok(Fleet {
        identities,
        key_ring: key_ring_from(&shared.signing_keys)?,
        tenant_id: shared.tenant_id,
        grpc_endpoint: shared.grpc_endpoint,
    })
}

async fn enroll_missing(
    config: &FleetConfig,
    tls: &EnrollTls,
    tokens: Vec<String>,
    first: usize,
    missing: usize,
    store: &Arc<Store>,
) -> anyhow::Result<Vec<EnrolledIdentity>> {
    let semaphore = Arc::new(Semaphore::new(config.concurrency.max(1)));
    let gate = (config.per_minute > 0).then(|| {
        let mut interval =
            tokio::time::interval(Duration::from_secs_f64(60.0 / f64::from(config.per_minute)));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Arc::new(tokio::sync::Mutex::new(interval))
    });
    let mut tasks = JoinSet::new();
    for offset in 0..missing {
        let semaphore = Arc::clone(&semaphore);
        let gate = gate.clone();
        let store = Arc::clone(store);
        let config = config.clone();
        let tls = tls.clone();
        let token = tokens.get(offset).cloned();
        let index = first + offset;
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .context("the enrollment limiter closed")?;
            let facts = EnrollFacts {
                hostname: format!("{}-{index:05}", config.hostname_prefix),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                agent_version: env!("CARGO_PKG_VERSION").to_owned(),
                restic_version: "0.19.1".to_owned(),
            };
            let entry = enroll_with_retry(&config, token, &facts, &tls, gate.as_deref()).await?;
            store.append(&entry)?;
            anyhow::Ok((offset, entry))
        });
    }
    let mut enrolled: Vec<Option<EnrolledIdentity>> = (0..missing).map(|_| None).collect();
    while let Some(joined) = tasks.join_next().await {
        let (offset, entry) = joined.context("an enrollment task panicked")??;
        enrolled[offset] = Some(entry);
    }
    enrolled
        .into_iter()
        .map(|entry| entry.ok_or_else(|| anyhow!("an enrollment is missing")))
        .collect()
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

/// Waits for the pacing gate, takes a token and enrolls. An HTTP 429 waits and tries again, with
/// a fresh slot of the gate.
async fn enroll_with_retry(
    config: &FleetConfig,
    mut token: Option<String>,
    facts: &EnrollFacts,
    tls: &EnrollTls,
    gate: Option<&tokio::sync::Mutex<tokio::time::Interval>>,
) -> anyhow::Result<EnrolledIdentity> {
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        if let Some(gate) = gate {
            gate.lock().await.tick().await;
        }
        if token.is_none() {
            token = Some(fresh_token(config, tls).await?);
        }
        let value = token.as_deref().unwrap_or_default();
        match enroll_tls(&config.enroll_url, value, facts, tls).await {
            Ok(identity) => return Ok(identity),
            Err(error) => {
                let limited = format!("{error:#}").contains("HTTP 429");
                last = Some(error);
                let wait = if limited {
                    Duration::from_secs(20)
                } else {
                    Duration::from_millis(250 * (1 << attempt))
                };
                tokio::time::sleep(wait).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("no enrollment attempt ran")))
        .with_context(|| format!("the enrollment of {} failed", facts.hostname))
}

async fn fresh_token(config: &FleetConfig, tls: &EnrollTls) -> anyhow::Result<String> {
    match &config.tokens {
        TokenSource::File(_) => bail!("the token file ran out of tokens"),
        TokenSource::Api { url, api_token } => {
            let api_tls = url.starts_with("https://").then_some(tls);
            create_enrollment_token(url, api_token, TOKEN_TTL_SECONDS, api_tls)
                .await
                .context("cannot create an enrollment token")
        }
    }
}

fn shared_from(entry: &EnrolledIdentity) -> Shared {
    Shared {
        tenant_id: entry.tenant_id.clone(),
        grpc_endpoint: entry.grpc_endpoint.clone(),
        signing_keys: entry
            .signing_keys
            .iter()
            .map(|key| SharedKey {
                key_id: key.key_id.clone(),
                public_key: key.public_key.clone(),
            })
            .collect(),
    }
}

fn key_ring_from(keys: &[SharedKey]) -> anyhow::Result<KeyRing> {
    let mut key_ring = KeyRing::new();
    for key in keys {
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
    Ok(key_ring)
}

/// The store directory has the layout of `--certs`: `ca.pem`, `agents.txt` and `clients/`.
struct Store {
    dir: Option<PathBuf>,
    list: Mutex<()>,
}

impl Store {
    fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            list: Mutex::new(()),
        }
    }

    fn append(&self, entry: &EnrolledIdentity) -> anyhow::Result<()> {
        use std::io::Write as _;
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let _guard = lock(&self.list);
        let clients = dir.join("clients");
        std::fs::create_dir_all(&clients)?;
        std::fs::write(
            clients.join(format!("{}.pem", entry.agent_id)),
            chain_pem(entry),
        )?;
        std::fs::write(
            clients.join(format!("{}.key", entry.agent_id)),
            &entry.key_pem,
        )?;
        let mut list = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("agents.txt"))?;
        writeln!(list, "{}", entry.agent_id)?;
        Ok(())
    }

    fn write_shared(&self, shared: Option<&Shared>) -> anyhow::Result<()> {
        let (Some(dir), Some(shared)) = (&self.dir, shared) else {
            return Ok(());
        };
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(SHARED_FILE), serde_json::to_vec_pretty(shared)?)?;
        Ok(())
    }
}

fn read_store(dir: &Path) -> anyhow::Result<Vec<AgentIdentity>> {
    if !dir.join("agents.txt").exists() {
        return Ok(Vec::new());
    }
    crate::identity::read_client_identities(dir)
}

fn read_shared(dir: &Path) -> anyhow::Result<Option<Shared>> {
    let path = dir.join(SHARED_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let shared = serde_json::from_slice(&text)
        .with_context(|| format!("{} is not valid", path.display()))?;
    Ok(Some(shared))
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

/// The certificate of the agent followed by its issuing CAs. The root is the trust anchor of the
/// control plane, so the chain leaves it out.
fn chain_pem(entry: &EnrolledIdentity) -> String {
    let intermediates = entry.ca_chain_der.len().saturating_sub(1);
    let mut pem = der_to_pem(&entry.certificate_der);
    for der in entry.ca_chain_der.iter().take(intermediates) {
        pem.push_str(&der_to_pem(der));
    }
    pem
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

    #[test]
    fn the_store_keeps_an_identity_for_the_next_run() {
        let dir = std::env::temp_dir().join(format!("rh-store-{}", std::process::id()));
        let store = Store::new(Some(dir.clone()));
        let entry = EnrolledIdentity {
            agent_id: "agent-a".to_owned(),
            tenant_id: "tenant".to_owned(),
            certificate_der: vec![1, 2, 3],
            ca_chain_der: Vec::new(),
            key_pem: "key".to_owned(),
            signing_key_update: Vec::new(),
            signing_keys: Vec::new(),
            signing_keys_signature: String::new(),
            grpc_endpoint: None,
        };

        store.append(&entry).expect("append");
        store
            .write_shared(Some(&shared_from(&entry)))
            .expect("shared");

        let identities = read_store(&dir).expect("identities");
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].agent_id, "agent-a");
        let shared = read_shared(&dir).expect("read").expect("shared");
        assert_eq!(shared.tenant_id, "tenant");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
