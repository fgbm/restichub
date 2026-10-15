//! The command bodies of the agent (FR-IN-07, FR-IN-08, FR-AG-01).

use std::io::Read as _;
use std::path::{Path, PathBuf};
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
use crate::service::{HostService, ServiceManager};
use crate::setup::{SetupReport, WaitOptions};

/// The exit code of a service start without a usable identity. The systemd unit does not restart
/// on it, because a restart cannot create the identity (FR-AG-01).
pub const EXIT_NO_IDENTITY: u8 = 78;

/// The identity is absent or damaged, so the service cannot run (FR-AG-01).
#[derive(Debug)]
pub struct NoIdentity {
    dir: PathBuf,
    cause: String,
}

impl std::fmt::Display for NoIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot load the agent identity in {}: {}; register the agent again with a new enrollment token",
            self.dir.display(),
            self.cause
        )
    }
}

impl std::error::Error for NoIdentity {}

/// Enrolls this host on a private runtime, then installs and starts the service and waits for the
/// connection (FR-IN-07).
///
/// `--no-service` stops after the enrollment. The restic download runs after the preflight and before the enrollment request, so a host
/// without restic and without network fails before the token is spent (ADR-0038, FR-IN-07).
///
/// # Errors
///
/// Returns an error when the runtime cannot start, from [`provision_and_register`], when the
/// service cannot be installed, or when the agent does not connect.
pub fn register(options: &RegisterOptions, dir: &Path) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    // The tree check runs first: the move of an old restic reads from it, so a directory that
    // another account can write must be refused before anything leaves it (T20, FR-IN-07).
    crate::preflight::prepare_directories(dir, &crate::paths::cache_dir())?;
    place_program()?;
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
    })?;
    let restic = ResticBinary::installed(&installer.layout)?;
    let report = start_after_register(
        options,
        dir,
        &HostService,
        restic.path(),
        WaitOptions::default(),
    )?;
    finish_setup(report.as_ref(), hostname().ok().as_deref())
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
    // The location rules run before the token is spent, so a refused location never leaves an
    // enrolled host without a service (T20, FR-IN-07).
    if !options.no_service {
        crate::service::check_install_location(restic.path())?;
    }
    register_async(options, dir, cache, &restic).await
}

/// Installs and starts the service after a successful enrollment, unless `--no-service` is set
/// (FR-IN-07, FR-IN-08).
///
/// A failed install keeps the identity and names the command to run next.
///
/// # Errors
///
/// Returns an error when the service cannot be installed or started.
pub fn start_after_register(
    options: &RegisterOptions,
    dir: &Path,
    manager: &dyn ServiceManager,
    restic: &Path,
    wait: WaitOptions,
) -> Result<Option<SetupReport>> {
    if options.no_service {
        return Ok(None);
    }
    crate::setup::install_and_wait(manager, dir, restic, wait)
        .map(Some)
        .context("the agent is registered, but the service is not ready; fix the cause and run: restichub-agent install-service")
}

/// Copies the running agent into the program directory and moves an old restic there (FR-IN-07).
fn place_program() -> Result<()> {
    let source = crate::self_install::running_image()?;
    let outcome = crate::self_install::prepare(
        &crate::self_install::Placement::host(),
        &source,
        &HostService,
    )?;
    if outcome.copied_agent {
        println!(
            "the agent is installed in {}",
            crate::paths::program_dir().display()
        );
    }
    if outcome.moved_restic {
        println!(
            "restic is moved into {}",
            crate::paths::restic_dir().display()
        );
    }
    Ok(())
}

/// Prints the final summary and fails when the agent is not connected.
fn finish_setup(report: Option<&SetupReport>, hostname: Option<&str>) -> Result<()> {
    let Some(report) = report else {
        println!("registered; the service is not installed (--no-service)");
        println!("{}", crate::setup::no_service_hint());
        return Ok(());
    };
    println!("{}", crate::setup::render_report(report, hostname));
    if report.connected() {
        Ok(())
    } else {
        bail!("the agent is registered but not connected yet")
    }
}

/// Installs and starts the service for the stored identity and waits for the connection
/// (FR-IN-08).
///
/// An installed service gets the new executable path and restarts. A host without an identity is
/// refused before restic is downloaded.
///
/// # Errors
///
/// Returns an error when the process is not elevated, the identity is absent, the service cannot be
/// installed, or the agent does not connect.
pub fn install_service(options: &ResticFileOptions) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    let dir = crate::paths::config_dir();
    crate::setup::require_identity(&dir)?;
    crate::preflight::prepare_directories(&dir, &crate::paths::cache_dir())?;
    place_program()?;
    let restic = ensure_restic(options)?;
    let report =
        crate::setup::install_and_wait(&HostService, &dir, restic.path(), WaitOptions::default())?;
    finish_setup(Some(&report), hostname().ok().as_deref())
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
    }
    Ok(())
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

/// Prints the identity, restic, the service and the connection without secrets (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the agent is not connected, so the exit code is not zero.
pub fn status(dir: &Path) -> Result<()> {
    let restic = ResticBinary::installed(&Layout::host())
        .ok()
        .and_then(|restic| {
            let version = restic.version().ok()?;
            Some((version, restic.path().display().to_string()))
        });
    let report = crate::setup::collect_status(&HostService, dir, restic);
    crate::setup::print_status(&report)
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
    let identity = StoredIdentity::load(dir).map_err(|error| {
        if !is_missing_or_damaged(&error) {
            return error;
        }
        let no_identity = NoIdentity {
            dir: dir.to_path_buf(),
            cause: format!("{error:#}"),
        };
        tracing::error!("{no_identity}");
        crate::connection::record_failure(dir, "", &no_identity.to_string());
        anyhow::Error::new(no_identity)
    })?;
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

/// A transient read error must not stop the service for good: exit code 78 tells systemd not to
/// restart it (FR-AG-01).
fn is_missing_or_damaged(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .is_none_or(|io| io.kind() == std::io::ErrorKind::NotFound)
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
    fn fr_ag_01_only_a_missing_or_damaged_identity_is_a_no_identity_error() {
        let missing = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::NotFound))
            .context("cannot read identity.json");
        let denied = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            .context("cannot read identity.json");
        let damaged = anyhow::anyhow!("bad identity.json");
        assert!(is_missing_or_damaged(&missing));
        assert!(is_missing_or_damaged(&damaged));
        assert!(!is_missing_or_damaged(&denied));
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
