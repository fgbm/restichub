//! The command bodies of the agent (FR-IN-07, FR-IN-08, FR-AG-01).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use anyhow::{Context, Result, bail};
use time::OffsetDateTime;

use crate::cli::{InstallOptions, RegisterOptions, ResticFileOptions, UpdateOptions};
use crate::enroll::{Client, Facts};
use crate::identity::{StoredIdentity, StoredSigningKeys, generate_key_and_csr};
use crate::machine_id::MachineId;
use crate::paths::UntrustedIdentity;
use crate::preflight::ChangeableIdentity;
use crate::restic::{Restic, ResticBinary};
use crate::restic_install::Installer;
use crate::restic_release::Supply;
use crate::restic_store::Layout;
use crate::service::{HostService, ServiceManager};
use crate::setup::{SetupReport, WaitOptions};

/// The exit code of a service start without a usable identity or restic. The service manager does
/// not restart on it, because only an administrator can fix them (FR-AG-01).
pub const EXIT_NO_IDENTITY: u8 = 78;

/// Tells whether the service stopped for a reason that only an administrator can fix.
#[must_use]
pub fn needs_administrator(error: &anyhow::Error) -> bool {
    error.downcast_ref::<NoIdentity>().is_some()
        || error.downcast_ref::<UnsafeIdentity>().is_some()
        || error.downcast_ref::<NoRestic>().is_some()
        || error.downcast_ref::<UnsafeProgram>().is_some()
}

/// Another account could change the agent program or restic, so the service does not run them
/// (T20). On Windows a DLL beside the program runs with the rights of the service.
#[derive(Debug)]
pub struct UnsafeProgram {
    cause: String,
}

impl std::fmt::Display for UnsafeProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "another account could change the agent program or restic: {}; the service stays stopped until an administrator runs `install-service` of the release package",
            self.cause
        )
    }
}

impl std::error::Error for UnsafeProgram {}

fn check_program(exe: &Path, restic: &Path) -> Result<(), UnsafeProgram> {
    crate::service::check_location(exe, restic).map_err(|error| UnsafeProgram {
        cause: format!("{error:#}"),
    })
}

/// Restic is absent or differs from its record, and the service never downloads it (ADR-0038).
#[derive(Debug)]
pub struct NoRestic {
    cause: String,
}

impl std::fmt::Display for NoRestic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "restic is not installed or differs from its record: {}; the service stays stopped until an administrator runs `restichub-agent install-service`, which installs restic and starts the service; on a host without the restic download, add `--restic-archive <ARCHIVE>` with the restic release archive for this host",
            self.cause
        )
    }
}

impl std::error::Error for NoRestic {}

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

/// Another account could change the identity, so the service does not use it (FR-AG-04, T20).
#[derive(Debug)]
pub struct UnsafeIdentity {
    dir: PathBuf,
    cause: String,
    kind: UnsafeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnsafeKind {
    Changeable,
    /// This process may not read an entry, so the check has no result.
    Unreadable,
}

impl std::fmt::Display for UnsafeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            UnsafeKind::Changeable => write!(
                f,
                "the agent does not use the identity in {}: {}; an administrator runs `restichub-agent install-service`, which repairs the access control; if it says that another account could change the identity, {}",
                self.dir.display(),
                self.cause,
                crate::preflight::REGISTER_AGAIN
            ),
            UnsafeKind::Unreadable => write!(
                f,
                "cannot check the identity in {}: {}; run the command as an administrator (Windows: an elevated PowerShell) or as root (Linux)",
                self.dir.display(),
                self.cause
            ),
        }
    }
}

impl std::error::Error for UnsafeIdentity {}

/// Refuses an identity that another account could change, before anything reads it (T20).
///
/// # Errors
///
/// Returns [`UnsafeIdentity`] with the entry that another account could change, or with the entry
/// that this process may not read.
pub fn check_identity(dir: &Path) -> Result<(), UnsafeIdentity> {
    crate::paths::check_identity_dir(dir).map_err(|error| unsafe_identity(dir, &error))
}

fn unsafe_identity(dir: &Path, error: &anyhow::Error) -> UnsafeIdentity {
    let denied = error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|io| io.kind() == std::io::ErrorKind::PermissionDenied);
    UnsafeIdentity {
        dir: dir.to_path_buf(),
        cause: format!("{error:#}"),
        kind: if denied {
            UnsafeKind::Unreadable
        } else {
            UnsafeKind::Changeable
        },
    }
}

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
    check_register_input(options, dir)?;
    // Every check and repair that can refuse runs before `--force` removes an identity that another
    // account could change, so a refusal leaves the old files in place (T20, FR-IN-07).
    prepare_program(dir, &HostService)?;
    // The tree check runs before the copy: the move of an old restic reads from it, so a directory
    // that another account can write must be refused before anything leaves it (T20, FR-IN-07).
    crate::preflight::prepare_directories(
        dir,
        &crate::paths::cache_dir(),
        untrusted(options),
        options.verbose,
    )?;
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
    crate::preflight::prepare_directories(dir, cache, untrusted(options), options.verbose)?;
    installer.ensure(supply).await?;
    let restic = ResticBinary::installed(&installer.layout)?;
    // The location rules run before the token is spent, so a refused location never leaves an
    // enrolled host without a service (T20, FR-IN-07).
    if !options.no_service {
        crate::service::check_install_location(restic.path())?;
    }
    if options.trust_server_scripts {
        let was_enabled = crate::scripts::ScriptStore::for_config_dir(dir, cache).mode()
            == crate::scripts::ScriptMode::Enabled;
        return enroll_with_trust_mode(
            dir,
            was_enabled,
            &crate::scripts::set_mode,
            register_async(options, dir, cache, &restic),
        )
        .await;
    }
    register_async(options, dir, cache, &restic).await
}

/// Writes the trust mode before the enrollment, so a mode file that the host cannot trust never
/// spends the token. A failed enrollment puts the earlier mode back (FR-HK-08, ADR-0037).
async fn enroll_with_trust_mode(
    dir: &Path,
    was_enabled: bool,
    write_mode: &dyn Fn(&Path, bool) -> Result<()>,
    enroll: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    let mode_file = crate::scripts::mode_file_for(dir);
    let existed = std::fs::symlink_metadata(&mode_file).is_ok();
    write_mode(dir, true).context(
        "the trust mode cannot be written, so the agent did not register and the token is not used",
    )?;
    let Err(error) = enroll.await else {
        println!("{}", trust_enabled_text(dir));
        return Ok(());
    };
    let reset = if !existed {
        std::fs::remove_file(&mode_file)
            .with_context(|| format!("cannot remove {}", mode_file.display()))
    } else if was_enabled {
        Ok(())
    } else {
        write_mode(dir, false)
    };
    match reset {
        Ok(()) => Err(error),
        Err(reset_error) => Err(error.context(format!(
            "the registration failed, and the trust mode stays on because it cannot be reset ({reset_error:#}); run: restichub-agent trust-server-scripts --disable"
        ))),
    }
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

/// Makes an open program directory again, checks it and reads the service state, so that the later
/// copy meets no refusal (FR-IN-07, T20).
fn prepare_program(config: &Path, manager: &dyn ServiceManager) -> Result<()> {
    let running = crate::self_install::running_image()?;
    // A running service and its restic keep the directory in use. The command restarts the service
    // at the end anyway.
    let rebuilt = crate::paths::rebuild_program_dir(
        config,
        running.path(),
        &mut || {
            if manager.state()?.running {
                manager.stop()?;
            }
            Ok(())
        },
        &mut |line| println!("{line}"),
    )?;
    crate::paths::ensure_program_dir(&crate::paths::restic_dir())?;
    crate::paths::ensure_program_dir(&crate::paths::program_dir())?;
    if rebuilt {
        // The installed service starts from this path also when a later check refuses the identity.
        crate::self_install::copy_agent(&crate::self_install::Placement::host(), &running)?;
        println!(
            "the agent is installed in {}",
            crate::paths::program_dir().display()
        );
    }
    manager
        .state()
        .context("cannot read the state of the agent service")?;
    Ok(())
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
        verify_moved_restic(&Layout::host())?;
        println!(
            "restic is moved into {}",
            crate::paths::restic_dir().display()
        );
    }
    Ok(())
}

/// The record keeps the hash, so the moved binary must still match it (T20).
fn verify_moved_restic(layout: &Layout) -> Result<()> {
    if layout.has_record() {
        layout
            .verify_installed()
            .context("restic differs from its record after the move")?;
    }
    Ok(())
}

const NOT_CONNECTED: &str = "the agent is registered but not connected yet; follow the next step above, then run: restichub-agent status";

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
        bail!("{NOT_CONNECTED}")
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
pub fn install_service(options: &InstallOptions) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    let dir = crate::paths::config_dir();
    prepare_program(&dir, &HostService)?;
    prepare_install(
        &dir,
        &crate::paths::cache_dir(),
        &HostService,
        options.verbose,
    )?;
    place_program()?;
    let restic = ensure_restic(&options.restic)?;
    let report =
        crate::setup::install_and_wait(&HostService, &dir, restic.path(), WaitOptions::default())?;
    finish_setup(Some(&report), hostname().ok().as_deref())
}

/// Only `register --force` replaces the identity, so only it may drop an identity that another
/// account could change; any other run leaves it as found and refuses (T20).
fn untrusted(options: &RegisterOptions) -> UntrustedIdentity {
    if options.force {
        UntrustedIdentity::Remove
    } else {
        UntrustedIdentity::Keep
    }
}

/// The identity load finishes a staged rotation, so the tree check and the repair run before it, as
/// in `register`: another account may have planted the staged files (T20).
fn prepare_install(
    dir: &Path,
    cache: &Path,
    manager: &dyn ServiceManager,
    verbose: bool,
) -> Result<()> {
    if let Err(error) =
        crate::preflight::prepare_directories(dir, cache, UntrustedIdentity::Keep, verbose)
    {
        return Err(match error.downcast::<ChangeableIdentity>() {
            Ok(changeable) => refuse_changeable_identity(&changeable, manager),
            Err(error) => error,
        });
    }
    crate::setup::require_identity(dir)?;
    Ok(())
}

/// The running service may talk to a replaced control plane, so it stops; it stays installed for
/// the new registration (T20).
fn refuse_changeable_identity(
    changeable: &ChangeableIdentity,
    manager: &dyn ServiceManager,
) -> anyhow::Error {
    let service = match manager.state() {
        Ok(state) if state.installed && state.running => match manager.stop() {
            Ok(()) => "; the running service is stopped and stays installed".to_owned(),
            Err(error) => format!(
                "; the running service did not stop ({error:#}), so stop it by hand: {STOP_COMMAND}"
            ),
        },
        Ok(_) => String::new(),
        Err(error) => format!(
            "; the state of the service cannot be read ({error:#}), so make sure that it is stopped: {STOP_COMMAND}"
        ),
    };
    anyhow::anyhow!(
        "the identity of this host could be changed by another account ({}), so the agent does not use it and the service is not installed{service}; {}",
        changeable.files(),
        crate::preflight::REGISTER_AGAIN
    )
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
    crate::preflight::prepare_directories(dir, cache, untrusted(options), options.verbose)?;

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
/// The agent reads the mode before each job, but reports it only when it connects. An idle running
/// service therefore restarts and the command waits until it is connected again, unless
/// `no_restart` is set. A service with a job in progress keeps running, because a restart would
/// stop the job.
///
/// # Errors
///
/// Returns an error when the process is not elevated, the setting file cannot be written, the
/// service cannot be restarted, or the agent does not connect again.
pub fn trust_server_scripts(enable: bool, no_restart: bool) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    apply_trust_mode(
        &HostService,
        &crate::paths::config_dir(),
        enable,
        no_restart,
        WaitOptions::default(),
        &crate::scripts::set_mode,
    )
}

#[cfg(windows)]
const RESTART_COMMAND: &str = "Restart-Service restichub-agent";
#[cfg(not(windows))]
const RESTART_COMMAND: &str = "systemctl restart restichub-agent";
#[cfg(windows)]
const STOP_COMMAND: &str = "Stop-Service restichub-agent";
#[cfg(not(windows))]
const STOP_COMMAND: &str = "systemctl stop restichub-agent";

fn apply_trust_mode(
    manager: &dyn ServiceManager,
    dir: &Path,
    enable: bool,
    no_restart: bool,
    wait: WaitOptions,
    write_mode: &dyn Fn(&Path, bool) -> Result<()>,
) -> Result<()> {
    write_mode(dir, enable)?;
    if enable {
        println!("{}", trust_enabled_text(dir));
    } else {
        println!(
            "Server scripts are disabled on this host ({}).",
            crate::scripts::mode_file_for(dir).display()
        );
    }
    if no_restart {
        println!(
            "the mode applies to the next job; the control plane shows it after the service connects again, for example after: {RESTART_COMMAND}"
        );
        return Ok(());
    }
    let state = manager.state()?;
    if !state.installed || !state.running {
        println!(
            "no agent service is running; the mode applies to the next job, and the control plane shows it when the service connects"
        );
        return Ok(());
    }
    if let Some(reason) = busy_reason(dir) {
        println!(
            "{reason}, so the service does not restart; the mode applies to the next job, and the control plane shows it after the service connects again, for example after the job ends and you run: {RESTART_COMMAND}"
        );
        return Ok(());
    }
    let since = OffsetDateTime::now_utc();
    manager.restart().with_context(|| {
        format!(
            "the mode is written and applies to the next job, but the service did not restart; restart it by hand: {RESTART_COMMAND}"
        )
    })?;
    match crate::connection::wait_for_connection(dir, since, wait.timeout, wait.poll) {
        crate::connection::Wait::Connected(_) => {
            println!("the agent reconnected; the control plane now sees the new state");
            Ok(())
        }
        crate::connection::Wait::TimedOut(_) => bail!(
            "the mode is written and applies to the next job, but the agent did not connect again after the restart; the control plane shows the mode after it connects: {NOT_CONNECTED}"
        ),
    }
}

/// Why a restart could stop a job, from the job journal that the service keeps. A journal that
/// cannot be read gives no proof that the service is idle.
fn busy_reason(dir: &Path) -> Option<String> {
    match crate::journal::FileJournal::load(dir.join(crate::journal::JOURNAL_FILE)) {
        Ok(journal) if journal.has_unfinished_job(now_timestamp()) => {
            Some("the job journal shows a job in progress".to_owned())
        }
        Ok(_) => None,
        Err(error) => Some(format!("the job journal cannot be read ({error:#})")),
    }
}

fn now_timestamp() -> prost_types::Timestamp {
    let now = OffsetDateTime::now_utc();
    prost_types::Timestamp {
        seconds: now.unix_timestamp(),
        nanos: now.nanosecond() as i32,
    }
}

fn trust_enabled_text(dir: &Path) -> String {
    format!(
        "Server scripts are enabled on this host ({}). The control plane can now run any command here as the agent account.",
        crate::scripts::mode_file_for(dir).display()
    )
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
    // The service never repairs: a connection with an identity that another account could have
    // replaced would trust that account's control plane (T20).
    if let Err(refused) = check_identity(dir) {
        tracing::error!("{refused}");
        return Err(anyhow::Error::new(refused));
    }
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
    if let Some(file) = crate::scripts::removed_allow_list_file(dir) {
        tracing::warn!(
            file = %file.display(),
            "the hook allow-list file is no longer used; move its hooks to server scripts"
        );
    }
    // Rebuild the key ring, so a job that the current key signed verifies (ADR-0026, ADR-0030).
    let mut keys = crate::keys::KeyState::load(&identity, dir)?;
    let cache = crate::paths::cache_dir();
    crate::preflight::prepare_cache(&cache)?;
    // An update can replace the agent without a new `register`, so a restic in the Windows
    // location of an earlier version moves first (ADR-0038).
    #[cfg(windows)]
    let migration =
        crate::self_install::migrate_restic_at_start(&crate::self_install::Placement::host());
    #[cfg(not(windows))]
    let migration = Ok(());
    // The service never downloads restic; an absent or changed binary stops the start (ADR-0038).
    let restic = installed_restic(&Layout::host(), migration, || {
        std::thread::sleep(std::time::Duration::from_secs(2));
    })?;
    // `register` and `install-service` check the location, but a service that an older build or
    // a manual copy installed starts without them.
    #[cfg(any(target_os = "linux", windows))]
    {
        let exe = std::env::current_exe().context("cannot find the agent program")?;
        if let Err(refused) = check_program(&exe, restic.path()) {
            tracing::error!("{refused}");
            return Err(anyhow::Error::new(refused));
        }
    }
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

/// A restart cannot bring restic back, so its absence stops the service for good (FR-AG-01).
///
/// `restic update` renames the new binary before it writes the record, so a start in between
/// sees a mismatch: `pause` waits, and a second reading decides. A move from an old location that
/// failed for a transient reason leaves the restart to the service manager, which moves again.
fn installed_restic(
    layout: &Layout,
    migration: Result<()>,
    pause: impl FnOnce(),
) -> Result<ResticBinary> {
    let read = || {
        ResticBinary::installed(layout).and_then(|restic| {
            crate::restic::check_hash(restic.path(), restic.expected_sha256()).map(|()| restic)
        })
    };
    let restic = match read() {
        Err(error) if is_missing_or_damaged(&error) => {
            pause();
            read()
        }
        first => first,
    };
    restic.map_err(|error| {
        if !is_missing_or_damaged(&error) {
            return error;
        }
        if let Err(moving) = migration
            && !is_missing_or_damaged(&moving)
        {
            return moving.context(format!("restic is not in place: {error:#}"));
        }
        let no_restic = NoRestic {
            cause: format!("{error:#}"),
        };
        tracing::error!("{no_restic}");
        anyhow::Error::new(no_restic)
    })
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

const EMPTY_TOKEN: &str = "the token is empty; create an enrollment token in the console and pass it with --token, --token-file or --token-stdin";

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
        bail!("{EMPTY_TOKEN}");
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

    mod trust_mode {
        use std::cell::{Cell, RefCell};
        use std::time::Duration;

        use prost_types::Timestamp;
        use restichub_proto::signing::JobJournal as _;

        use super::*;
        use crate::journal::{FileJournal, JOURNAL_FILE};
        use crate::service::{ServiceState, StartType};

        /// A service double that records the calls in order. Its state is fixed; a restart writes
        /// the connection status like a service that connected, when `connects` is set.
        struct Fake {
            state: ServiceState,
            dir: PathBuf,
            connects: bool,
            restart_fails: bool,
            events: RefCell<Vec<String>>,
        }

        impl Fake {
            fn new(dir: &Path, state: ServiceState) -> Self {
                Self {
                    state,
                    dir: dir.to_path_buf(),
                    connects: true,
                    restart_fails: false,
                    events: RefCell::new(Vec::new()),
                }
            }

            fn events(&self) -> Vec<String> {
                self.events.borrow().clone()
            }
        }

        impl ServiceManager for Fake {
            fn install_and_start(&self, _restic: &Path) -> Result<()> {
                self.events.borrow_mut().push("start".to_owned());
                Ok(())
            }

            fn state(&self) -> Result<ServiceState> {
                Ok(self.state)
            }

            fn stop(&self) -> Result<()> {
                Ok(())
            }

            fn restart(&self) -> Result<()> {
                self.events.borrow_mut().push("restart".to_owned());
                if self.restart_fails {
                    bail!("cannot stop the service");
                }
                if self.connects {
                    crate::connection::record_connected(&self.dir, "https://cp.example:8443");
                }
                Ok(())
            }
        }

        const RUNNING: ServiceState = ServiceState {
            installed: true,
            running: true,
            start_type: StartType::Automatic,
        };

        const STOPPED: ServiceState = ServiceState {
            installed: true,
            running: false,
            start_type: StartType::Automatic,
        };

        fn quick() -> WaitOptions {
            WaitOptions {
                timeout: Duration::from_millis(100),
                poll: Duration::from_millis(10),
            }
        }

        fn config_dir(name: &str) -> PathBuf {
            let base = std::env::temp_dir().join(format!(
                "restichub-agent-trustmode-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            let dir = base.join("config");
            std::fs::create_dir_all(&dir).expect("config dir");
            dir
        }

        /// Writes the mode file without the trust check, which needs root or Administrators.
        fn write_mode(dir: &Path, enabled: bool) -> Result<()> {
            let value = if enabled { "enabled" } else { "disabled" };
            std::fs::write(
                crate::scripts::mode_file_for(dir),
                format!("server_scripts = {value}\n"),
            )?;
            Ok(())
        }

        fn refuse_mode(_dir: &Path, _enabled: bool) -> Result<()> {
            bail!("the agent would not trust the file")
        }

        fn mode_text(dir: &Path) -> String {
            std::fs::read_to_string(crate::scripts::mode_file_for(dir)).unwrap_or_default()
        }

        fn cleanup(dir: &Path) {
            let _ = std::fs::remove_dir_all(dir.parent().expect("parent"));
        }

        fn apply(fake: &Fake, dir: &Path, enable: bool, no_restart: bool) -> Result<()> {
            apply_trust_mode(fake, dir, enable, no_restart, quick(), &write_mode)
        }

        fn accept_job(dir: &Path, finished: bool) {
            let mut journal = FileJournal::load(dir.join(JOURNAL_FILE)).expect("journal");
            let expires = Timestamp {
                seconds: OffsetDateTime::now_utc().unix_timestamp() + 3600,
                nanos: 0,
            };
            journal.record_accepted("job-1", expires);
            if finished {
                journal.record_completed(
                    "job-1",
                    restichub_proto::agent::v1::JobResult {
                        job_id: "job-1".to_owned(),
                        ..Default::default()
                    },
                );
            }
            journal.persist().expect("persist");
        }

        #[test]
        fn fr_hk_08_trust_enable_restarts_an_idle_service_and_waits_for_the_connection() {
            let dir = config_dir("restart");
            accept_job(&dir, true);
            let fake = Fake::new(&dir, RUNNING);
            apply(&fake, &dir, true, false).expect("apply");
            assert!(mode_text(&dir).contains("server_scripts = enabled"));
            assert_eq!(fake.events(), ["restart"]);
            cleanup(&dir);
        }

        #[test]
        fn fr_hk_08_trust_change_does_not_restart_a_service_with_a_job_in_progress() {
            let dir = config_dir("busy");
            accept_job(&dir, false);
            let fake = Fake::new(&dir, RUNNING);
            apply(&fake, &dir, false, false).expect("apply");
            assert!(mode_text(&dir).contains("server_scripts = disabled"));
            assert!(fake.events().is_empty());
            cleanup(&dir);
        }

        #[test]
        fn fr_hk_08_a_journal_that_cannot_be_read_does_not_prove_an_idle_service() {
            let dir = config_dir("bad-journal");
            std::fs::write(dir.join(JOURNAL_FILE), b"not json").expect("write");
            let fake = Fake::new(&dir, RUNNING);
            apply(&fake, &dir, true, false).expect("apply");
            assert!(fake.events().is_empty());
            cleanup(&dir);
        }

        #[test]
        fn fr_hk_08_trust_enable_fails_when_the_agent_does_not_reconnect() {
            let dir = config_dir("no-reconnect");
            let mut fake = Fake::new(&dir, RUNNING);
            fake.connects = false;
            let error = format!(
                "{:#}",
                apply(&fake, &dir, true, false).expect_err("no connection")
            );
            assert!(error.contains("did not connect again"), "{error}");
            assert!(error.contains("the mode is written"), "{error}");
            cleanup(&dir);
        }

        #[test]
        fn fr_hk_08_a_failed_restart_says_that_the_mode_is_written() {
            let dir = config_dir("restart-fails");
            let mut fake = Fake::new(&dir, RUNNING);
            fake.restart_fails = true;
            let error = format!(
                "{:#}",
                apply(&fake, &dir, false, false).expect_err("restart fails")
            );
            assert!(error.contains("the mode is written"), "{error}");
            assert!(error.contains(RESTART_COMMAND), "{error}");
            assert!(mode_text(&dir).contains("server_scripts = disabled"));
            cleanup(&dir);
        }

        #[test]
        fn fr_hk_08_trust_change_does_not_restart_a_stopped_or_absent_service() {
            let dir = config_dir("stopped");
            for state in [STOPPED, ServiceState::NOT_INSTALLED] {
                let fake = Fake::new(&dir, state);
                apply(&fake, &dir, false, false).expect("apply");
                assert!(fake.events().is_empty());
            }
            assert!(mode_text(&dir).contains("server_scripts = disabled"));
            cleanup(&dir);
        }

        #[test]
        fn fr_hk_08_no_restart_leaves_the_service_alone() {
            let dir = config_dir("no-restart");
            let fake = Fake::new(&dir, RUNNING);
            apply(&fake, &dir, true, true).expect("apply");
            assert!(fake.events().is_empty());
            cleanup(&dir);
        }

        #[tokio::test]
        async fn fr_hk_08_register_writes_the_trust_mode_before_the_enrollment() {
            let dir = config_dir("register");
            let enroll = async {
                assert!(mode_text(&dir).contains("server_scripts = enabled"));
                Ok(())
            };
            enroll_with_trust_mode(&dir, false, &write_mode, enroll)
                .await
                .expect("register");
            assert!(mode_text(&dir).contains("server_scripts = enabled"));
            cleanup(&dir);
        }

        #[tokio::test]
        async fn fr_hk_08_a_mode_that_cannot_be_written_does_not_spend_the_token() {
            let dir = config_dir("register-refused");
            let enrolled = Cell::new(false);
            let enroll = async {
                enrolled.set(true);
                Ok(())
            };
            let error = enroll_with_trust_mode(&dir, false, &refuse_mode, enroll)
                .await
                .expect_err("the mode cannot be written");
            assert!(
                format!("{error:#}").contains("token is not used"),
                "{error:#}"
            );
            assert!(!enrolled.get(), "the enrollment ran");
            cleanup(&dir);
        }

        #[tokio::test]
        async fn fr_hk_08_a_failed_enrollment_puts_the_earlier_mode_back() {
            let dir = config_dir("register-fails");
            let failed = || async { Err(anyhow::anyhow!("the control plane refused the token")) };
            enroll_with_trust_mode(&dir, false, &write_mode, failed())
                .await
                .expect_err("the enrollment fails");
            assert!(!crate::scripts::mode_file_for(&dir).exists());
            write_mode(&dir, false).expect("write");
            enroll_with_trust_mode(&dir, false, &write_mode, failed())
                .await
                .expect_err("the enrollment fails");
            assert!(mode_text(&dir).contains("server_scripts = disabled"));
            cleanup(&dir);
        }
    }

    #[test]
    fn fr_in_07_a_moved_restic_that_differs_from_its_record_fails() {
        let base =
            std::env::temp_dir().join(format!("restichub-agent-moved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let layout = Layout::new(base.join("bin"), base.join("config"));
        std::fs::create_dir_all(layout.bin_dir()).expect("bin");
        std::fs::create_dir_all(base.join("config")).expect("config");
        std::fs::write(layout.binary_path(), b"restic").expect("restic");
        let wrong = "a".repeat(64);
        std::fs::write(
            layout.record_path(),
            format!(r#"{{"version":"0.19.1","sha256":"{wrong}"}}"#),
        )
        .expect("record");

        let error = verify_moved_restic(&layout).expect_err("mismatch");

        assert!(format!("{error:#}").contains("differs from its record after the move"));
        assert!(layout.binary_path().exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_01_a_missing_or_changed_restic_stops_the_service_for_an_administrator() {
        use sha2::Digest as _;
        let base =
            std::env::temp_dir().join(format!("restichub-agent-no-restic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let layout = Layout::new(base.join("bin"), base.join("config"));
        std::fs::create_dir_all(layout.bin_dir()).expect("bin");
        std::fs::create_dir_all(base.join("config")).expect("config");
        let refused = |state: &str, cause: String| {
            let error = installed_restic(&layout, Ok(()), || {}).expect_err(state);
            assert!(needs_administrator(&error), "{state}: {error:#}");
            assert_eq!(
                format!("{error:#}"),
                format!(
                    "restic is not installed or differs from its record: {cause}; the service stays stopped until an administrator runs `restichub-agent install-service`, which installs restic and starts the service; on a host without the restic download, add `--restic-archive <ARCHIVE>` with the restic release archive for this host"
                ),
                "{state}"
            );
        };
        let hash = |bytes: &[u8]| crate::restic_release::hex(&sha2::Sha256::digest(bytes));

        refused(
            "no record",
            format!(
                "restic is not installed: cannot read {}; run `restichub-agent restic update`: {}",
                layout.record_path().display(),
                std::io::Error::from_raw_os_error(2)
            ),
        );
        let record = |bytes: &[u8]| {
            std::fs::write(
                layout.record_path(),
                format!(r#"{{"version":"0.19.1","sha256":"{}"}}"#, hash(bytes)),
            )
            .expect("record");
        };
        record(b"restic");
        refused(
            "no binary",
            format!(
                "the restic binary {} is missing; run `restichub-agent restic update`",
                layout.binary_path().display()
            ),
        );
        std::fs::write(layout.binary_path(), b"changed").expect("restic");
        refused(
            "a changed binary",
            format!(
                "the restic binary does not match the recorded SHA-256: expected {}, got {}",
                hash(b"restic"),
                hash(b"changed")
            ),
        );
        record(b"changed");
        installed_restic(&layout, Ok(()), || {}).expect("the recorded binary passes");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_01_the_service_refuses_a_program_that_another_account_could_change() {
        // The temporary directory belongs to the test account, not to root or Administrators.
        let base = std::env::temp_dir().join(format!(
            "restichub-agent-open-program-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("base");
        let exe = base.join("restichub-agent");
        let restic = base.join(crate::restic::RESTIC_FILE_NAME);
        std::fs::write(&exe, b"agent").expect("agent");
        std::fs::write(&restic, b"restic").expect("restic");

        let error = anyhow::Error::new(check_program(&exe, &restic).expect_err("an open program"));
        assert!(needs_administrator(&error), "{error:#}");
        let text = format!("{error:#}");
        assert!(
            text.starts_with("another account could change the agent program or restic: "),
            "{text}"
        );
        assert!(
            text.contains("runs `install-service` of the release package"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_01_a_start_during_the_restic_swap_reads_restic_again() {
        use sha2::Digest as _;
        let base = std::env::temp_dir().join(format!(
            "restichub-agent-swap-window-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let layout = Layout::new(base.join("bin"), base.join("config"));
        std::fs::create_dir_all(layout.bin_dir()).expect("bin");
        std::fs::create_dir_all(base.join("config")).expect("config");
        let record = |bytes: &[u8]| {
            let hash = crate::restic_release::hex(&sha2::Sha256::digest(bytes));
            std::fs::write(
                layout.record_path(),
                format!(r#"{{"version":"0.19.1","sha256":"{hash}"}}"#),
            )
            .expect("record");
        };
        record(b"old");
        std::fs::write(layout.binary_path(), b"new").expect("the renamed binary");

        installed_restic(&layout, Ok(()), || record(b"new"))
            .expect("the record that the update writes next passes");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_01_a_transient_failure_to_move_an_old_restic_restarts_the_service() {
        let base = std::env::temp_dir().join(format!(
            "restichub-agent-move-failed-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let layout = Layout::new(base.join("bin"), base.join("config"));
        let failed_move = |kind: std::io::ErrorKind| {
            Err(anyhow::Error::new(std::io::Error::from(kind))
                .context("cannot copy the old restic"))
        };

        let locked = installed_restic(
            &layout,
            failed_move(std::io::ErrorKind::PermissionDenied),
            || {},
        )
        .expect_err("restic is not in place");
        assert!(!needs_administrator(&locked), "{locked:#}");
        assert!(
            format!("{locked:#}").starts_with("restic is not in place: "),
            "{locked:#}"
        );
        let gone = installed_restic(&layout, failed_move(std::io::ErrorKind::NotFound), || {})
            .expect_err("restic is not in place");
        assert!(needs_administrator(&gone), "{gone:#}");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn planted_rotation(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "restichub-agent-install-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("ResticHub").join("Agent");
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        std::fs::write(dir.join("agent-key.pem"), b"key").expect("cannot write the key");
        std::fs::write(dir.join("agent-key.pem.next"), b"planted").expect("cannot write");
        std::fs::write(dir.join("rotation.next"), b"").expect("cannot write the marker");
        (base, dir)
    }

    #[cfg(unix)]
    #[test]
    fn fr_ag_04_install_service_prepares_the_directory_before_it_loads_the_identity() {
        use std::os::unix::fs::PermissionsExt as _;
        let (base, dir) = planted_rotation("order");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
            .expect("cannot set the mode");
        let error = prepare_install(&dir, &base.join("cache"), &StopCounter::new(false), false)
            .expect_err("no identity");
        assert!(
            format!("{error:#}").contains("no valid agent identity"),
            "{error:#}"
        );
        let mode = std::fs::metadata(&dir)
            .expect("cannot read the directory")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "the directory was not prepared first");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(windows)]
    #[test]
    fn fr_ag_04_install_service_refuses_an_open_directory_before_it_loads_the_identity() {
        let (base, dir) = planted_rotation("order");
        crate::winsec::tests::grant(&dir, "(OI)(CI)(WD)").expect("cannot add the entry");
        let error = prepare_install(&dir, &dir.join("cache"), &StopCounter::new(false), false)
            .expect_err("an open directory");
        assert!(
            format!("{error:#}").contains("cannot prepare the directory"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(dir.join("agent-key.pem")).expect("the key must stay"),
            b"key"
        );
        assert!(dir.join("agent-key.pem.next").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A service double that counts the stops; a stop makes it not running.
    struct StopCounter {
        state: std::cell::Cell<crate::service::ServiceState>,
        stops: std::cell::Cell<u32>,
    }

    impl StopCounter {
        fn new(running: bool) -> Self {
            Self {
                state: std::cell::Cell::new(crate::service::ServiceState {
                    installed: true,
                    running,
                    start_type: crate::service::StartType::Automatic,
                }),
                stops: std::cell::Cell::new(0),
            }
        }
    }

    impl ServiceManager for StopCounter {
        fn install_and_start(&self, _restic: &Path) -> Result<()> {
            bail!("a refused identity must not install the service")
        }

        fn state(&self) -> Result<crate::service::ServiceState> {
            Ok(self.state.get())
        }

        fn stop(&self) -> Result<()> {
            self.stops.set(self.stops.get() + 1);
            let mut state = self.state.get();
            state.running = false;
            self.state.set(state);
            Ok(())
        }

        fn restart(&self) -> Result<()> {
            bail!("a refused identity must not restart the service")
        }
    }

    #[test]
    fn fr_ag_04_install_service_refuses_an_identity_that_another_account_could_change_and_stops_the_service()
     {
        let chain = PathBuf::from("Agent").join(crate::identity::CA_CHAIN_FILE);
        let changeable = ChangeableIdentity {
            paths: vec![chain.clone()],
        };
        let manager = StopCounter::new(true);
        let error = format!("{:#}", refuse_changeable_identity(&changeable, &manager));
        assert_eq!(manager.stops.get(), 1, "the running service must stop");
        assert!(error.contains(&chain.display().to_string()), "{error}");
        assert!(error.contains("the service is not installed"), "{error}");
        assert!(
            error.contains("the running service is stopped and stays installed"),
            "{error}"
        );
        assert!(
            error.contains("register --force --url <URL> --token <TOKEN>"),
            "{error}"
        );

        let error = format!("{:#}", refuse_changeable_identity(&changeable, &manager));
        assert_eq!(manager.stops.get(), 1, "a stopped service is left alone");
        assert!(!error.contains("running service"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn fr_ag_04_the_service_does_not_start_with_an_identity_that_another_account_could_change() {
        use std::os::unix::fs::PermissionsExt as _;
        let (base, dir) = planted_rotation("unsafe-run");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
            .expect("cannot set the mode");
        let error = run(&dir, None).expect_err("the service must refuse");
        assert!(needs_administrator(&error), "{error:#}");
        let text = format!("{error:#}");
        assert!(text.contains("install-service"), "{text}");
        assert!(text.contains("register --force"), "{text}");
        assert!(
            dir.join("agent-key.pem.next").exists(),
            "the refusal reads nothing and finishes no rotation"
        );
        let report = crate::setup::collect_status(&StopCounter::new(false), &dir, None);
        let identity = report.identity.expect_err("status must report the refusal");
        assert!(identity.contains("does not use the identity"), "{identity}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_check_without_the_rights_asks_for_an_administrator() {
        let denied = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            .context("cannot open C:\\ProgramData\\ResticHub");
        let text = unsafe_identity(Path::new("Agent"), &denied).to_string();
        assert!(
            text.contains("run the command as an administrator"),
            "{text}"
        );
        assert!(!text.contains("register"), "{text}");

        let open = anyhow::anyhow!("Agent grants a write access to an account");
        let text = unsafe_identity(Path::new("Agent"), &open).to_string();
        assert!(text.contains("register --force"), "{text}");
    }

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
    fn an_empty_token_error_tells_what_to_do() {
        let options = RegisterOptions {
            token: Some("  \n".to_owned()),
            ..RegisterOptions::default()
        };
        let error = resolve_token(&options)
            .expect_err("an empty token")
            .to_string();
        assert!(error.contains("--token-file"), "{error}");
        assert!(error.contains("console"), "{error}");
    }

    #[test]
    fn a_not_connected_error_names_the_next_command() {
        assert!(NOT_CONNECTED.contains("restichub-agent status"));
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
