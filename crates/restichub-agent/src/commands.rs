//! The command bodies of the agent (FR-IN-07, FR-IN-08, FR-AG-01).

use std::io::Read as _;
use std::path::Path;
use std::sync::mpsc::Receiver;

use anyhow::{Context, Result, bail};
use time::OffsetDateTime;

use crate::cli::{RegisterOptions, ResticFileOptions, UpdateOptions};
use crate::enroll::{Client, Facts};
use crate::identity::{StoredIdentity, StoredSigningKeys, generate_key_and_csr};
use crate::machine_id::MachineId;
use crate::restic::{Restic, ResticBinary};
use crate::restic_install::Installer;
use crate::restic_release::Supply;
use crate::restic_store::Layout;

/// Enrolls this host on a private runtime (FR-IN-07).
///
/// The restic download runs after the preflight and before the enrollment request, so a host
/// without restic and without network fails before the token is spent (ADR-0038, FR-IN-07).
///
/// # Errors
///
/// Returns an error when the runtime cannot start, or from [`provision_and_register`].
pub fn register(options: &RegisterOptions, dir: &Path) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    let installer = Installer::host()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the runtime")?;
    runtime.block_on(async {
        let supply = crate::restic_install::supply(&options.restic)?;
        provision_and_register(
            options,
            dir,
            &crate::paths::cache_dir(),
            &installer,
            &supply,
        )
        .await
    })
}

/// Checks the input, installs restic when it is absent and enrolls this host (FR-IN-07, ADR-0038).
///
/// # Errors
///
/// Returns an error from the checks, from the restic install, or from [`register_async`].
pub async fn provision_and_register(
    options: &RegisterOptions,
    dir: &Path,
    cache: &Path,
    installer: &Installer<'_>,
    supply: &Supply,
) -> Result<()> {
    check_register_input(options, dir)?;
    crate::preflight::prepare_directories(dir, cache)?;
    installer.ensure(supply).await?;
    let restic = ResticBinary::installed(&installer.layout)?;
    register_async(options, dir, cache, &restic).await
}

/// Installs restic when it is absent and returns it (ADR-0038).
///
/// # Errors
///
/// Returns an error when the runtime cannot start or the install fails.
pub fn ensure_restic(options: &ResticFileOptions) -> Result<ResticBinary> {
    let installer = Installer::host()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the runtime")?;
    runtime.block_on(async {
        let supply = crate::restic_install::supply(options)?;
        installer.ensure(&supply).await
    })?;
    ResticBinary::installed(&installer.layout)
}

/// Downloads the latest restic and replaces the installed one (ADR-0038).
///
/// The installed service restarts afterwards, unless the options say otherwise.
///
/// # Errors
///
/// Returns an error when the process is not elevated or the update fails. The old binary stays.
pub fn restic_update(options: &UpdateOptions) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    crate::paths::ensure_config_dir(&crate::paths::config_dir())?;
    let installer = Installer::host()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the runtime")?;
    runtime.block_on(async {
        let supply = crate::restic_install::supply(&options.restic)?;
        installer
            .update_with(&supply, options.allow_downgrade)
            .await
    })?;
    if options.no_restart {
        println!("restart the agent service so that it uses the new restic");
    } else if !crate::service::restart_if_installed()? {
        println!("no agent service is installed; nothing to restart");
    }
    Ok(())
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
/// ready, when restic is not the recorded binary, or when the control plane refuses the enrollment.
pub async fn register_async(
    options: &RegisterOptions,
    dir: &Path,
    cache: &Path,
    restic: &dyn Restic,
) -> Result<()> {
    check_register_input(options, dir)?;
    let url = options.url.clone().unwrap_or_default();
    let token = resolve_token(options)?;
    crate::preflight::prepare_directories(dir, cache)?;

    let facts = Facts {
        hostname: hostname()?,
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        agent_version: restichub_common::BUILD_VERSION.to_owned(),
        restic_version: restic_version(restic)?,
        machine_id: MachineId::read(),
    };

    let (generated, csr_der) = generate_key_and_csr()?;
    let client = Client::new(options.ca_file.as_deref())?;
    crate::preflight::check_clock(OffsetDateTime::now_utc(), client.server_time(&url).await)?;
    let enrolled = client.enroll(&url, &token, &facts, &csr_der).await?;
    // The response is checked before any file is written (ADR-0026, FR-AG-04).
    crate::verify::check_enrollment(&generated.public_key, &enrolled, OffsetDateTime::now_utc())?;

    let reenrolled = enrolled.reenrolled;
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
    if reenrolled {
        println!("the agent is registered and replaced the existing host record");
    } else {
        println!("the agent is registered");
    }
    print_identity(&identity)
}

/// Turns the local trust mode for server scripts on or off (FR-HK-08, ADR-0037).
///
/// # Errors
///
/// Returns an error when the setting file cannot be written or the host would not trust it, for
/// example when the command does not run as root or Administrator.
pub fn trust_server_scripts(enable: bool) -> Result<()> {
    let dir = crate::paths::config_dir();
    crate::scripts::set_mode(&dir, enable)?;
    let file = crate::scripts::mode_file_for(&dir);
    if enable {
        println!(
            "Server scripts are enabled on this host ({}). The control plane can now run code here as the agent account.",
            file.display()
        );
    } else {
        println!(
            "Server scripts are disabled on this host ({}).",
            file.display()
        );
    }
    Ok(())
}

fn check_register_input(options: &RegisterOptions, dir: &Path) -> Result<()> {
    if options.url.is_none() {
        bail!("--url is required for register");
    }
    check_token_sources(options)?;
    if StoredIdentity::exists(dir) && !options.force {
        bail!(
            "an agent identity already exists in {}; pass --force to register again",
            dir.display()
        );
    }
    Ok(())
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
/// Returns an error when the identity is absent, restic is not the recorded binary, or the runtime
/// cannot start.
pub fn run(dir: &Path, shutdown: Option<Receiver<()>>) -> Result<()> {
    let identity = StoredIdentity::load(dir)
        .with_context(|| format!("no agent identity in {}", dir.display()))?;
    // Rebuild the key ring, so a job that the current key signed verifies (ADR-0026, ADR-0030).
    let mut keys = crate::keys::KeyState::load(&identity, dir)?;
    let cache = crate::paths::cache_dir();
    crate::preflight::prepare_cache(&cache)?;
    // The service never downloads restic; an absent or changed binary stops the start (ADR-0038).
    let restic = ResticBinary::installed(&Layout::host())?;
    restic.verify()?;
    let restic_version = restic.version().unwrap_or_default();
    // The job journal lives in the configuration directory; the runner uses the verified restic
    // binary (ADR-0020, ADR-0038).
    let jobs = crate::jobs::Jobs::open(
        restic.path().to_path_buf(),
        restic.expected_sha256().to_owned(),
        cache,
        dir,
    )?
    .with_record(Layout::host());
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

fn check_token_sources(options: &RegisterOptions) -> Result<()> {
    let sources = usize::from(options.token.is_some())
        + usize::from(options.token_file.is_some())
        + usize::from(options.token_stdin);
    match sources {
        0 => bail!("give the token with --token, --token-file or --token-stdin"),
        1 => Ok(()),
        _ => bail!("give exactly one of --token, --token-file or --token-stdin"),
    }
}

fn resolve_token(options: &RegisterOptions) -> Result<String> {
    check_token_sources(options)?;
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
