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
use crate::program_repair::HostRecord;
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
        || error.downcast_ref::<UnsafeTree>().is_some()
}

/// A directory that lost the entry of a local administrator at this start holds something that the
/// repair of `install-service` refuses, so the service does not start (ADR-0048, T20).
///
/// The service cannot repair it: the account could write there while the entry existed, and only
/// an administrator can check that content.
#[derive(Debug)]
pub struct UnsafeTree {
    cause: String,
}

impl std::fmt::Display for UnsafeTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the agent does not start: a folder that File Explorer opened to a local administrator holds something that the agent refuses: {}; an administrator checks the folder, removes what this message names, and runs `restichub-agent install-service`",
            self.cause
        )
    }
}

impl std::error::Error for UnsafeTree {}

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
                "the agent does not use the identity in {}: {}; an administrator runs `restichub-agent install-service`, which removes the access that File Explorer gave to a local administrator and repairs the access control; if it says that another account could change the identity, {}",
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
    refuse_running_job(&HostService, dir)?;
    // Every check and repair that can refuse runs before `--force` removes an identity that another
    // account could change, so a refusal leaves the old files in place (T20, FR-IN-07).
    let program = crate::program_repair::prepare(dir, &HostService)?;
    // The token is spent only for a tree that the service may start from.
    if !options.no_service {
        program.check_before_start()?;
    }
    // The tree check runs before the copy: the move of an old restic reads from it, so a directory
    // that another account can write must be refused before anything leaves it (T20, FR-IN-07).
    let cache = crate::paths::cache_dir();
    let prepared = PreparedDirectories::prepare(options, dir, &cache)?;
    place_program()?;
    let installer = Installer::host()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the runtime")?;
    let registered = runtime.block_on(async {
        let supply = crate::restic_install::supply(&options.restic)?;
        provision_and_register(options, &prepared, &installer, &supply).await
    });
    if program.rebuilt.replaced() {
        registered.context("the service stays stopped until this command succeeds")?;
    } else {
        registered?;
    }
    let restic = ResticBinary::installed(&installer.layout)?;
    if !options.no_service {
        program
            .start_service_from_tree(&HostRecord, crate::paths::check_program_files)
            .context(REGISTERED_NOT_STARTED)?;
    }
    let report = start_after_register(
        options,
        dir,
        &HostService,
        restic.path(),
        WaitOptions::default(),
    )?;
    if let Some(warning) = program.warning() {
        println!("{warning}");
    }
    finish_setup(report.as_ref(), hostname().ok().as_deref())
}

const REGISTERED_NOT_STARTED: &str = "the agent is registered, but the service is not started; fix the cause and run: restichub-agent install-service";

/// The configuration and cache directories after the register checks and repairs, which run once
/// and before the token is spent (FR-IN-07, T20).
pub struct PreparedDirectories<'a> {
    dir: &'a Path,
    cache: &'a Path,
}

impl<'a> PreparedDirectories<'a> {
    /// Repairs and proves both directories (see [`crate::preflight::prepare_directories`]).
    ///
    /// # Errors
    ///
    /// Returns the error of [`crate::preflight::prepare_directories`].
    pub fn prepare(options: &RegisterOptions, dir: &'a Path, cache: &'a Path) -> Result<Self> {
        crate::preflight::prepare_directories(dir, cache, untrusted(options), options.verbose)?;
        Ok(Self { dir, cache })
    }
}

/// Installs restic when it is absent and enrolls this host (FR-IN-07, ADR-0038).
///
/// The caller has checked the input and prepared the directories.
///
/// # Errors
///
/// Returns an error from the restic install or from the enrollment.
pub async fn provision_and_register(
    options: &RegisterOptions,
    prepared: &PreparedDirectories<'_>,
    installer: &Installer<'_>,
    supply: &Supply,
) -> Result<()> {
    let PreparedDirectories { dir, cache } = *prepared;
    installer.ensure(supply).await.context(
        "restic is not installed, so the agent did not register and the token is not used",
    )?;
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
            enroll_prepared(options, prepared, &restic),
        )
        .await;
    }
    enroll_prepared(options, prepared, &restic).await
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
/// Returns an error when the process is not elevated, a job runs in the service, the identity is
/// absent, the service cannot be installed, or the agent does not connect.
pub fn install_service(options: &InstallOptions) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    let dir = crate::paths::config_dir();
    // Before the repair, which stops the service too (FR-AG-10, FR-IN-08).
    refuse_running_job(&HostService, &dir)?;
    let program = crate::program_repair::prepare(&dir, &HostService)?;
    prepare_install(
        &dir,
        &crate::paths::cache_dir(),
        &HostService,
        options.verbose,
    )?;
    place_program()?;
    let restic = ensure_restic(&options.restic)?;
    program.start_service_from_tree(&HostRecord, crate::paths::check_program_files)?;
    let report =
        crate::setup::install_and_wait(&HostService, &dir, restic.path(), WaitOptions::default())?;
    if let Some(warning) = program.warning() {
        println!("{warning}");
    }
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
    if !options.no_restart {
        // The restart at the end would stop a job and its restic (FR-AG-10).
        refuse_running_job(&HostService, &crate::paths::config_dir())?;
    }
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
    let token = resolve_token(options)?;
    let prepared = PreparedDirectories::prepare(options, dir, cache)?;
    enroll(options, &prepared, restic, &token).await
}

async fn enroll_prepared(
    options: &RegisterOptions,
    prepared: &PreparedDirectories<'_>,
    restic: &dyn Restic,
) -> Result<()> {
    let token = resolve_token(options)?;
    enroll(options, prepared, restic, &token).await
}

async fn enroll(
    options: &RegisterOptions,
    prepared: &PreparedDirectories<'_>,
    restic: &dyn Restic,
    token: &str,
) -> Result<()> {
    let dir = prepared.dir;
    let url = options.url.clone().unwrap_or_default();
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
    let enrolled = client.enroll(&url, token, &facts, &csr_der).await?;
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

/// Refuses a command that would stop the service while it runs a job, because a stop cuts the job
/// short and restic can lose its repository lock (FR-AG-10, FR-IN-08).
///
/// A stopped service is no refusal: it runs no job, and a journal entry that a crash left behind
/// must not block the command.
fn refuse_running_job(manager: &dyn ServiceManager, dir: &Path) -> Result<()> {
    if !manager.state()?.running {
        return Ok(());
    }
    let Some(reason) = busy_reason(dir) else {
        return Ok(());
    };
    bail!(
        "cannot stop the service: {reason}; wait for the job to end, or stop the service by hand, then run the command again"
    )
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

/// Stops and removes the operating system service (FR-IN-08).
///
/// # Errors
///
/// Returns an error when a job runs in the service, or the service cannot be removed.
pub fn uninstall_service() -> Result<()> {
    crate::preflight::ensure_elevated()?;
    refuse_running_job(&HostService, &crate::paths::config_dir())?;
    crate::service::uninstall()
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
    // A non-elevated token of an administrator holds Administrators as deny-only, so File Explorer
    // asks before it opens an agent folder and then adds an entry for that account. The service
    // removes that entry and runs the checks and the repair that `install-service` runs for the
    // changed levels (ADR-0048, T20).
    #[cfg(windows)]
    let healed_program = match crate::winrepair::heal_admin_entries(dir) {
        Ok(program) => program,
        Err(error) => {
            let refused = UnsafeTree {
                cause: format!("{error:#}"),
            };
            tracing::error!("{refused}");
            return Err(anyhow::Error::new(refused));
        }
    };
    #[cfg(not(windows))]
    let healed_program = false;
    if healed_program && let Err(refused) = crate::paths::check_program_files() {
        // A library that the account planted while the entry existed loads before `main`.
        let refused = UnsafeProgram {
            cause: format!("{refused:#}"),
        };
        tracing::error!("{refused}");
        return Err(anyhow::Error::new(refused));
    }
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
    // A cache that another account could change would restart the service forever with the exit
    // code of an ordinary error, so on Windows it stops like an identity that another account could
    // change. Linux keeps the ordinary error.
    #[cfg(windows)]
    if let Err(error) = crate::preflight::prepare_cache(&cache) {
        let refused = unsafe_identity(&cache, &error);
        tracing::error!("{refused}");
        return Err(anyhow::Error::new(refused));
    }
    #[cfg(not(windows))]
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
mod tests;
