//! Command line of the emulator.
//!
//! The workspace has no argument parser, so this module reads the arguments
//! itself and prints the usage on `--help`. The option names are stable for the
//! bench scripts.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use ed25519_dalek::VerifyingKey;
use restichub_proto::signing::KeyRing;
use tokio::sync::watch;

use crate::agent::{AgentConfig, Backoff, run_agent};
use crate::executor::{Executor, FakeRestic, Simulated, SimulatedProfile};
use crate::identity::{self, AgentIdentity, TestPki};

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
  --executor NAME     simulated (default) or fake-restic
  --fake-restic PATH  path to the restichub-fake-restic binary
  --tenant ID         tenant identifier (default tenant-1)
  --key-id ID         job signing key identifier (default job-key-1)
  --key HEX           job signing public key, 32 bytes in hex
  --step-seconds SEC  wall time of one simulated step (default 0.1)
  --progress-ticks N  progress messages for each simulated step (default 0)
  --duration SEC      stop after SEC seconds (default: run until Ctrl+C)
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
            "--server" => options.server = take_value(args, &mut index, &name)?,
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

    let (ca_pem, identities) = resolve_identity(options)?;
    let key_ring = resolve_key_ring(options)?;
    let executor: Arc<dyn Executor> = match options.executor {
        ExecutorKind::Simulated => Arc::new(Simulated {
            profile: options.profile,
        }),
        ExecutorKind::FakeRestic => {
            let program = options
                .fake_restic
                .clone()
                .ok_or_else(|| anyhow!("--executor fake-restic needs --fake-restic PATH"))?;
            Arc::new(FakeRestic { program })
        }
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut tasks = Vec::with_capacity(identities.len());
    for identity in identities {
        let config = Arc::new(AgentConfig {
            tenant_id: options.tenant.clone(),
            server_url: options.server.clone(),
            server_name: options.server_name.clone(),
            ca_pem: ca_pem.clone(),
            identity,
            key_ring: key_ring.clone(),
            executor: Arc::clone(&executor),
            backoff: Backoff::default(),
        });
        tasks.push(tokio::spawn(run_agent(config, shutdown_rx.clone())));
    }
    println!("started {} agents against {}", tasks.len(), options.server);

    match options.duration {
        Some(seconds) => {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(seconds)) => {}
                result = tokio::signal::ctrl_c() => result.context("cannot listen for Ctrl+C")?,
            }
        }
        None => tokio::signal::ctrl_c()
            .await
            .context("cannot listen for Ctrl+C")?,
    }

    let _ = shutdown_tx.send(true);
    for task in tasks {
        let _ = task.await;
    }
    Ok(())
}

fn resolve_identity(options: &Options) -> anyhow::Result<(String, Vec<AgentIdentity>)> {
    if let Some(dir) = &options.certs {
        let ca = match &options.ca {
            Some(path) => std::fs::read_to_string(path)?,
            None => identity::read_ca(dir)?,
        };
        return Ok((ca, identity::read_client_identities(dir)?));
    }
    let pki = TestPki::generate(options.agents)?;
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
