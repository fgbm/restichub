//! Command line of the emulator.
//!
//! The workspace has no argument parser, so this module reads the arguments
//! itself and prints the usage on `--help`. The option names are stable for the
//! bench scripts.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use ed25519_dalek::VerifyingKey;
use restichub_proto::signing::KeyRing;

use crate::agent::Backoff;
use crate::executor::{Executor, FakeRestic, Profiled, ProfiledProfile, SimulatedProfile};
use crate::fleet::{FleetConfig, TokenSource, enroll_fleet};
use crate::identity::{self, AgentIdentity, TestPki};
use crate::profile::{JobDuration, LoadProfile};
use crate::runner::{LoadRun, RunConfig};

const USAGE: &str = "\
restichub-emulator - ResticHub agent emulator for load tests

USAGE:
  restichub-emulator [OPTIONS]

OPTIONS:
  --agents N          number of emulated agents (default 1)
  --server URL        control plane URL (default https://127.0.0.1:50051)
  --server-name NAME  TLS server name (default localhost)
  --ca FILE           certificate authority that verifies the server
  --certs DIR         directory with ca.pem, agents.txt and clients/
  --write-certs DIR   make the test certificates in DIR and exit
  --enroll-url URL    enroll the agents at this enrollment listener (HTTPS) instead of
                      using test certificates; needs --ca and one token source
  --token-file FILE   one enrollment token on each line, at least one for each agent
  --api-url URL       make the tokens through the API of this control plane
  --api-token-file F  file with the API token that --api-url uses
  --enroll-concurrency N  enrollments at the same time (default 16)
  --enroll-per-minute N  enrollments started in a minute (default 9; the control plane
                      accepts 10 from one address, 0 turns the pacing off)
  --identities DIR    keep the enrolled identities in DIR; a later run reuses them and
                      enrolls only the missing agents
  --enroll-only       enroll the agents and exit
  --executor NAME     simulated (default) or fake-restic
  --fake-restic PATH  path to the restichub-fake-restic binary
  --tenant ID         tenant identifier (default tenant-1)
  --key-id ID         job signing key identifier (default job-key-1)
  --key HEX           job signing public key, 32 bytes in hex
  --step-seconds SEC  wall time of one simulated step (default 0.1)
  --progress-ticks N  progress messages for each simulated step (default 0)
  --duration SEC      stop after SEC seconds (default: the profile length, or run until Ctrl+C)
  --profile FILE      load profile file (JSON); overrides --agents, --duration,
                      --step-seconds and --progress-ticks
  --metrics-addr ADDR Prometheus endpoint address, or none (default 127.0.0.1:9100)
  --summary FILE      write the final JSON summary to FILE
  --help              print this help";

/// Which executor the agents use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorKind {
    Simulated,
    FakeRestic,
}

/// Values read from the command line.
#[derive(Debug)]
pub struct Options {
    pub agents: usize,
    pub server: String,
    pub server_name: String,
    pub ca: Option<PathBuf>,
    pub certs: Option<PathBuf>,
    pub write_certs: Option<PathBuf>,
    pub executor: ExecutorKind,
    pub fake_restic: Option<PathBuf>,
    pub tenant: String,
    pub key_id: String,
    pub key: Option<String>,
    pub profile: SimulatedProfile,
    pub duration: Option<u64>,
    pub profile_file: Option<PathBuf>,
    pub metrics_addr: String,
    pub summary: Option<PathBuf>,
    pub enroll_url: Option<String>,
    pub token_file: Option<PathBuf>,
    pub api_url: Option<String>,
    pub api_token_file: Option<PathBuf>,
    pub enroll_concurrency: usize,
    pub enroll_per_minute: u32,
    pub identities: Option<PathBuf>,
    pub enroll_only: bool,
    server_set: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            agents: 1,
            server: "https://127.0.0.1:50051".to_owned(),
            server_name: identity::SERVER_NAME.to_owned(),
            ca: None,
            certs: None,
            write_certs: None,
            executor: ExecutorKind::Simulated,
            fake_restic: None,
            tenant: identity::TENANT_ID.to_owned(),
            key_id: identity::KEY_ID.to_owned(),
            key: None,
            profile: SimulatedProfile::default(),
            duration: None,
            profile_file: None,
            metrics_addr: "127.0.0.1:9100".to_owned(),
            summary: None,
            enroll_url: None,
            token_file: None,
            api_url: None,
            api_token_file: None,
            enroll_concurrency: 16,
            enroll_per_minute: 9,
            identities: None,
            enroll_only: false,
            server_set: false,
        }
    }
}

fn take_value(args: &[String], index: &mut usize, name: &str) -> anyhow::Result<String> {
    if let Some((_, value)) = args[*index].split_once('=') {
        return Ok(value.to_owned());
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| anyhow!("missing value for {name}"))
}

fn parse(args: &[String]) -> anyhow::Result<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        let name = args[index].split('=').next().unwrap_or_default().to_owned();
        match name.as_str() {
            "--agents" => options.agents = take_value(args, &mut index, &name)?.parse()?,
            "--server" => {
                options.server = take_value(args, &mut index, &name)?;
                options.server_set = true;
            }
            "--enroll-url" => options.enroll_url = Some(take_value(args, &mut index, &name)?),
            "--token-file" => {
                options.token_file = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--api-url" => options.api_url = Some(take_value(args, &mut index, &name)?),
            "--api-token-file" => {
                options.api_token_file = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--enroll-concurrency" => {
                options.enroll_concurrency = take_value(args, &mut index, &name)?.parse()?;
            }
            "--enroll-per-minute" => {
                options.enroll_per_minute = take_value(args, &mut index, &name)?.parse()?;
            }
            "--identities" => {
                options.identities = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--enroll-only" => options.enroll_only = true,
            "--server-name" => options.server_name = take_value(args, &mut index, &name)?,
            "--ca" => options.ca = Some(PathBuf::from(take_value(args, &mut index, &name)?)),
            "--certs" => options.certs = Some(PathBuf::from(take_value(args, &mut index, &name)?)),
            "--write-certs" => {
                options.write_certs = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--executor" => {
                options.executor = match take_value(args, &mut index, &name)?.as_str() {
                    "simulated" => ExecutorKind::Simulated,
                    "fake-restic" => ExecutorKind::FakeRestic,
                    other => bail!("unknown executor: {other}"),
                };
            }
            "--fake-restic" => {
                options.fake_restic = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--tenant" => options.tenant = take_value(args, &mut index, &name)?,
            "--key-id" => options.key_id = take_value(args, &mut index, &name)?,
            "--key" => options.key = Some(take_value(args, &mut index, &name)?),
            "--step-seconds" => {
                options.profile.step_seconds = take_value(args, &mut index, &name)?.parse()?;
            }
            "--progress-ticks" => {
                options.profile.progress_ticks = take_value(args, &mut index, &name)?.parse()?;
            }
            "--duration" => options.duration = Some(take_value(args, &mut index, &name)?.parse()?),
            "--profile" => {
                options.profile_file = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--metrics-addr" => options.metrics_addr = take_value(args, &mut index, &name)?,
            "--summary" => {
                options.summary = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
        index += 1;
    }
    Ok(options)
}

/// Reads the arguments, runs the emulator and returns when it stops.
///
/// # Errors
///
/// Returns an error for a bad argument, a connection failure or a file error.
pub async fn run_from_env() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = parse(&args)?;
    run(&options).await
}

async fn run(options: &Options) -> anyhow::Result<()> {
    if let Some(dir) = &options.write_certs {
        let pki = TestPki::generate(options.agents)?;
        pki.write(dir)?;
        println!(
            "wrote {} agent identities and the authority to {}",
            options.agents,
            dir.display()
        );
        return Ok(());
    }

    let profile = resolve_profile(options)?;
    let agents = profile.agents;
    let run_seconds = match options.duration {
        Some(seconds) => Some(Duration::from_secs(seconds)),
        None if options.profile_file.is_some() => Some(profile.duration()),
        None => None,
    };
    let mut server_url = options.server.clone();
    let mut tenant_id = options.tenant.clone();
    let (ca_pem, identities, key_ring) = match &options.enroll_url {
        Some(url) => {
            let fleet = enroll_agents(options, url, agents).await?;
            if let Some(endpoint) = fleet.grpc_endpoint.filter(|_| !options.server_set) {
                server_url = endpoint;
            }
            tenant_id = fleet.tenant_id;
            println!("enrolled {} agents at {url}", fleet.identities.len());
            if options.enroll_only {
                return Ok(());
            }
            (fleet.ca_pem, fleet.identities, fleet.key_ring)
        }
        None => {
            let (ca_pem, identities) = resolve_identity(options, agents)?;
            (ca_pem, identities, resolve_key_ring(options)?)
        }
    };
    let executor: Arc<dyn Executor> = match options.executor {
        ExecutorKind::Simulated => Arc::new(Profiled {
            profile: ProfiledProfile::from_profile(&profile),
        }),
        ExecutorKind::FakeRestic => {
            let program = options
                .fake_restic
                .clone()
                .ok_or_else(|| anyhow!("--executor fake-restic needs --fake-restic PATH"))?;
            Arc::new(FakeRestic { program })
        }
    };
    let metrics_addr = resolve_metrics_addr(options)?;

    let config = RunConfig {
        profile,
        server_url: server_url.clone(),
        server_name: options.server_name.clone(),
        ca_pem,
        tenant_id,
        key_ring,
        identities,
        executor,
        backoff: Backoff::default(),
        metrics_addr,
    };
    let run = LoadRun::start(config).await?;
    if let Some(addr) = run.metrics_addr() {
        println!("metrics on http://{addr}/metrics");
    }
    println!("started {agents} agents against {server_url}");

    match run_seconds {
        Some(seconds) => {
            tokio::select! {
                () = tokio::time::sleep(seconds) => {}
                result = tokio::signal::ctrl_c() => result.context("cannot listen for Ctrl+C")?,
            }
        }
        None => tokio::signal::ctrl_c()
            .await
            .context("cannot listen for Ctrl+C")?,
    }

    let summary = run.stop().await;
    if let Some(path) = &options.summary {
        let text =
            serde_json::to_string_pretty(&summary).context("cannot serialize the summary")?;
        std::fs::write(path, text)
            .with_context(|| format!("cannot write the summary {}", path.display()))?;
        println!("wrote the summary to {}", path.display());
    }
    println!(
        "{}",
        serde_json::to_string(&summary).context("cannot serialize the summary")?
    );
    Ok(())
}

struct EnrolledFleet {
    ca_pem: String,
    identities: Vec<AgentIdentity>,
    key_ring: KeyRing,
    tenant_id: String,
    grpc_endpoint: Option<String>,
}

async fn enroll_agents(
    options: &Options,
    url: &str,
    agents: usize,
) -> anyhow::Result<EnrolledFleet> {
    let ca_path = options.ca.as_ref().ok_or_else(|| {
        anyhow!("--enroll-url needs --ca FILE with the root CA of the installation")
    })?;
    let ca_pem = std::fs::read_to_string(ca_path)
        .with_context(|| format!("cannot read the CA file {}", ca_path.display()))?;
    let tokens = match (&options.token_file, &options.api_url) {
        (Some(path), None) => TokenSource::File(path.clone()),
        (None, Some(api_url)) => {
            let path = options
                .api_token_file
                .as_ref()
                .ok_or_else(|| anyhow!("--api-url needs --api-token-file FILE"))?;
            let api_token = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read the API token file {}", path.display()))?;
            TokenSource::Api {
                url: api_url.clone(),
                api_token: api_token.trim().to_owned(),
            }
        }
        _ => bail!("--enroll-url needs exactly one of --token-file and --api-url"),
    };
    let fleet = enroll_fleet(&FleetConfig {
        agents,
        enroll_url: url.to_owned(),
        tokens,
        ca_pem: ca_pem.clone(),
        server_name: options.server_name.clone(),
        concurrency: options.enroll_concurrency,
        per_minute: options.enroll_per_minute,
        hostname_prefix: "emulator".to_owned(),
        store: options.identities.clone(),
    })
    .await?;
    Ok(EnrolledFleet {
        ca_pem,
        identities: fleet.identities,
        key_ring: fleet.key_ring,
        tenant_id: fleet.tenant_id,
        grpc_endpoint: fleet.grpc_endpoint,
    })
}

fn resolve_profile(options: &Options) -> anyhow::Result<LoadProfile> {
    if let Some(path) = &options.profile_file {
        return LoadProfile::load(path);
    }
    let step_seconds = options.profile.step_seconds;
    let progress_rate = if step_seconds > 0.0 {
        f64::from(options.profile.progress_ticks) / step_seconds
    } else {
        0.0
    };
    let profile = LoadProfile {
        agents: options.agents,
        duration_seconds: options.duration.map_or(60.0, |seconds| seconds as f64),
        job_duration: JobDuration {
            min_seconds: step_seconds,
            max_seconds: step_seconds,
        },
        progress_rate,
        ..LoadProfile::default()
    };
    profile.validate()?;
    Ok(profile)
}

fn resolve_metrics_addr(options: &Options) -> anyhow::Result<Option<SocketAddr>> {
    if options.metrics_addr == "none" {
        return Ok(None);
    }
    let addr = options
        .metrics_addr
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid metrics address: {}", options.metrics_addr))?;
    Ok(Some(addr))
}

fn resolve_identity(
    options: &Options,
    agents: usize,
) -> anyhow::Result<(String, Vec<AgentIdentity>)> {
    if let Some(dir) = &options.certs {
        let ca = match &options.ca {
            Some(path) => std::fs::read_to_string(path)?,
            None => identity::read_ca(dir)?,
        };
        return Ok((ca, identity::read_client_identities(dir)?));
    }
    let pki = TestPki::generate(agents)?;
    let ca = match &options.ca {
        Some(path) => std::fs::read_to_string(path)?,
        None => pki.ca_pem().to_owned(),
    };
    Ok((ca, pki.agents().to_vec()))
}

fn resolve_key_ring(options: &Options) -> anyhow::Result<KeyRing> {
    let Some(hex) = &options.key else {
        return Ok(identity::key_ring());
    };
    let key = VerifyingKey::from_bytes(&decode_hex(hex)?)
        .map_err(|error| anyhow!("invalid job signing key: {error}"))?;
    let mut ring = KeyRing::new();
    ring.insert(options.key_id.clone(), key)
        .map_err(|error| anyhow!("cannot insert the job signing key: {error:?}"))?;
    Ok(ring)
}

fn decode_hex(hex: &str) -> anyhow::Result<[u8; 32]> {
    if hex.len() != 64 {
        bail!("the job signing key must be 64 hex characters");
    }
    let mut bytes = [0u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair)?;
        bytes[index] = u8::from_str_radix(text, 16)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default_options() {
        let options = parse(&[]).expect("default options");

        assert_eq!(options.agents, 1);
        assert_eq!(options.executor, ExecutorKind::Simulated);
        assert_eq!(options.tenant, identity::TENANT_ID);
    }

    #[test]
    fn parses_an_inline_and_a_separate_value() {
        let args = vec![
            "--agents=50".to_owned(),
            "--server".to_owned(),
            "https://example:50051".to_owned(),
            "--executor".to_owned(),
            "fake-restic".to_owned(),
        ];
        let options = parse(&args).expect("options");

        assert_eq!(options.agents, 50);
        assert_eq!(options.server, "https://example:50051");
        assert_eq!(options.executor, ExecutorKind::FakeRestic);
    }

    #[test]
    fn rejects_an_unknown_argument() {
        let args = vec!["--nope".to_owned()];

        assert!(parse(&args).is_err());
    }

    #[test]
    fn decodes_a_job_signing_key() {
        let hex = "07".repeat(32);

        assert_eq!(decode_hex(&hex).expect("hex"), [7u8; 32]);
    }

    #[test]
    fn rejects_a_short_key() {
        assert!(decode_hex("07").is_err());
    }
}
