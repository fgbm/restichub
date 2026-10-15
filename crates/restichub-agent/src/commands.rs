//! The command bodies of the agent (FR-IN-07, FR-IN-08, FR-AG-01).

use std::io::Read as _;
use std::path::Path;
use std::sync::mpsc::Receiver;

use anyhow::{Context, Result, bail};
use time::OffsetDateTime;

use crate::cli::RegisterOptions;
use crate::enroll::{Client, Facts};
use crate::identity::{StoredIdentity, StoredSigningKeys, generate_key_and_csr};
use crate::restic::{Restic, ResticBinary};

/// Enrolls this host on a private runtime (FR-IN-07).
///
/// # Errors
///
/// Returns an error when the runtime cannot start, or from [`register_async`].
pub fn register(options: &RegisterOptions, dir: &Path) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    let restic = ResticBinary::locate()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the runtime")?;
    runtime.block_on(register_async(
        options,
        dir,
        &crate::paths::cache_dir(),
        &restic,
    ))
}

/// Enrolls this host and writes the identity into `dir` (FR-IN-07).
///
/// The one-time token is read from the given source and never stored or logged (ADR-0032). The
/// directories are made and proven writable, and the restic binary is verified, before the network
/// is used, so a failure never spends the token (FR-IN-07, FR-AG-11).
///
/// # Errors
///
/// Returns an error when the identity already exists without `--force`, when a directory is not
/// ready, when restic is not the pinned binary, or when the control plane refuses the enrollment.
pub async fn register_async(
    options: &RegisterOptions,
    dir: &Path,
    cache: &Path,
    restic: &dyn Restic,
) -> Result<()> {
    let url = options
        .url
        .clone()
        .context("--url is required for register")?;
    let token = resolve_token(options)?;
    if StoredIdentity::exists(dir) && !options.force {
        bail!(
            "an agent identity already exists in {}; pass --force to register again",
            dir.display()
        );
    }
    crate::preflight::prepare_directories(dir, cache)?;

    let facts = Facts {
        hostname: hostname()?,
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        restic_version: restic_version(restic)?,
    };

    let (generated, csr_der) = generate_key_and_csr()?;
    let client = Client::new(options.ca_file.as_deref())?;
    let enrolled = client.enroll(&url, &token, &facts, &csr_der).await?;
    // The response is checked before any file is written (ADR-0026, FR-AG-04).
    crate::verify::check_enrollment(&generated.public_key, &enrolled, OffsetDateTime::now_utc())?;

    let identity = StoredIdentity {
        agent_id: enrolled.agent_id,
        tenant_id: enrolled.tenant_id,
        cp_url: url,
        grpc_endpoint: enrolled.grpc_endpoint,
        key_pem: generated.key_pem,
        certificate_der: enrolled.certificate_der,
        ca_chain_der: enrolled.ca_chain_der,
        signing_keys: StoredSigningKeys {
            keys: enrolled.signing_keys,
            signature: enrolled.signing_keys_signature,
        },
    };
    identity.save(dir)?;
    println!("the agent is registered");
    print_identity(&identity)
}

/// Prints the identity facts without secrets (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the identity is absent or the certificate is not valid.
pub fn status(dir: &Path) -> Result<()> {
    let identity = StoredIdentity::load(dir)
        .with_context(|| format!("no agent identity in {}", dir.display()))?;
    print_identity(&identity)
}

/// Loads the identity and serves the protocol stream until the shutdown signal (FR-AG-01, FR-ID-07).
///
/// The agent holds no timer that starts a job (ADR-0002).
///
/// # Errors
///
/// Returns an error when the identity is absent, restic is not the pinned binary, or the runtime
/// cannot start.
pub fn run(dir: &Path, shutdown: Option<Receiver<()>>) -> Result<()> {
    let identity = StoredIdentity::load(dir)
        .with_context(|| format!("no agent identity in {}", dir.display()))?;
    // Rebuild the key ring, so a job that the current key signed verifies (ADR-0026, ADR-0030).
    let mut keys = crate::keys::KeyState::load(&identity, dir)?;
    let cache = crate::paths::cache_dir();
    crate::preflight::prepare_cache(&cache)?;
    let restic = ResticBinary::locate()?;
    restic.verify()?;
    let restic_version = restic.version().unwrap_or_default();
    // The job journal lives in the configuration directory; the runner uses the verified restic
    // binary (ADR-0020, ADR-0031).
    let jobs = crate::jobs::Jobs::open(
        restic.path().to_path_buf(),
        crate::restic::EXPECTED_SHA256.to_owned(),
        cache,
        dir,
    )?;
    tracing::info!(
        agent_id = %identity.agent_id,
        keys = identity.signing_keys.keys.len(),
        "the agent identity is loaded"
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot start the runtime")?;
    runtime.block_on(crate::stream::serve(
        &identity,
        dir,
        &mut keys,
        &restic_version,
        &jobs,
        shutdown,
    ))
}

fn print_identity(identity: &StoredIdentity) -> Result<()> {
    let expiry = identity.certificate_expiry()?;
    println!("agent id:            {}", identity.agent_id);
    println!("tenant id:           {}", identity.tenant_id);
    println!("certificate expires: {expiry}");
    println!("control plane url:   {}", identity.cp_url);
    println!(
        "grpc endpoint:       {}",
        identity.grpc_endpoint.as_deref().unwrap_or("(not set)")
    );
    Ok(())
}

fn restic_version(restic: &dyn Restic) -> Result<String> {
    restic.verify()?;
    restic.version()
}

fn resolve_token(options: &RegisterOptions) -> Result<String> {
    let sources = usize::from(options.token.is_some())
        + usize::from(options.token_file.is_some())
        + usize::from(options.token_stdin);
    match sources {
        0 => bail!("give the token with --token, --token-file or --token-stdin"),
        1 => {}
        _ => bail!("give exactly one of --token, --token-file or --token-stdin"),
    }
    let raw = if let Some(token) = &options.token {
        token.clone()
    } else if let Some(path) = &options.token_file {
        std::fs::read_to_string(path)
            .with_context(|| format!("cannot read the token file {}", path.display()))?
    } else {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("cannot read the token from the standard input")?;
        text
    };
    let token = raw.trim().to_owned();
    if token.is_empty() {
        bail!("the token is empty");
    }
    Ok(token)
}

#[cfg(windows)]
fn hostname() -> Result<String> {
    std::env::var("COMPUTERNAME").context("the environment holds no COMPUTERNAME")
}

#[cfg(not(windows))]
fn hostname() -> Result<String> {
    for path in ["/proc/sys/kernel/hostname", "/etc/hostname"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            let name = text.trim();
            if !name.is_empty() {
                return Ok(name.to_owned());
            }
        }
    }
    bail!("cannot read the host name")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn the_token_sources_are_mutually_exclusive() {
        let options = RegisterOptions {
            token: Some("re_x".to_owned()),
            token_file: Some(std::path::PathBuf::from("/tmp/token")),
            ..RegisterOptions::default()
        };
        assert!(resolve_token(&options).is_err());
    }

    #[test]
    fn a_missing_token_source_is_refused() {
        assert!(resolve_token(&RegisterOptions::default()).is_err());
    }

    #[test]
    fn an_inline_token_is_trimmed() {
        let options = RegisterOptions {
            token: Some("  re_x\n".to_owned()),
            ..RegisterOptions::default()
        };
        assert_eq!(resolve_token(&options).expect("token"), "re_x");
    }
}
