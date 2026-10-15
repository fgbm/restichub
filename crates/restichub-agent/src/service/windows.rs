//! The Windows service of the agent (FR-AG-01, FR-IN-08, AC-IN-03).

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

pub const SERVICE_NAME: &str = "restichub-agent";

/// The Win32 error `ERROR_SERVICE_DOES_NOT_EXIST`, which means the service is not installed.
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

/// How long a restart waits for the service to stop. A running job gets its stop grace first.
const STOP_WAIT: Duration = Duration::from_secs(120);

define_windows_service!(ffi_service_main, service_main);

/// Runs the agent under the service control manager when the SCM started this process (FR-AG-01).
///
/// A process that the SCM did not start gives an error at once, so the command line runs.
///
/// # Errors
///
/// Returns the dispatcher error when the process is not a service.
pub fn run_as_service() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = run_service() {
        tracing::error!(%error, "the service stopped with an error");
    }
}

fn run_service() -> Result<()> {
    // A service has no console, and `CTRL_BREAK` reaches a child only through one (FR-AG-10).
    if let Err(error) = crate::exec::allocate_hidden_console() {
        tracing::warn!(%error, "cannot allocate the console; a cancel falls back to the forced kill");
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let handler = move |event| match event {
        ServiceControl::Stop => {
            let _ = sender.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, handler)?;
    status_handle.set_service_status(status(
        ServiceState::Running,
        ServiceControlAccept::STOP,
        ServiceExitCode::NO_ERROR,
    ))?;
    let result = crate::commands::run(&crate::paths::config_dir(), Some(receiver));
    // A failing run leaves a non-zero service exit code, so the SCM and an operator see it.
    let exit_code = match result {
        Ok(()) => ServiceExitCode::NO_ERROR,
        Err(_) => ServiceExitCode::Win32(1),
    };
    status_handle.set_service_status(status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit_code,
    ))?;
    result
}

fn status(
    state: ServiceState,
    accept: ServiceControlAccept,
    exit_code: ServiceExitCode,
) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

/// Writes and starts the Windows service (FR-IN-08, AC-IN-03).
///
/// An existing service is updated instead of created, so the command is idempotent. The update
/// points the service at the new executable path, sets the recovery actions and reports a start
/// failure.
///
/// # Errors
///
/// Returns an error when the service cannot be created, updated or started, usually without
/// administrator rights.
pub fn install(restic: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("cannot find the agent executable")?;
    // A binary that another account can replace defeats the service hardening (T20).
    super::location::check(&exe, restic)?;
    // The default restore root gets the access of SYSTEM and Administrators only (FR-RS-03).
    crate::restore_roots::ensure_default_root()?;
    if let Some((root, reason)) = crate::restore_roots::rejected_roots(&crate::paths::config_dir())
        .into_iter()
        .next()
    {
        anyhow::bail!("the restore root {} is not safe: {reason}", root.display());
    }
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("cannot open the service manager; run as an administrator")?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("ResticHub Agent"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![OsString::from("run")],
        dependencies: vec![],
        // A missing account runs the service as LocalSystem (FR-AG-01).
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::CHANGE_CONFIG | ServiceAccess::START;
    let service = match manager.create_service(&info, access) {
        Ok(service) => service,
        Err(error) => {
            tracing::debug!(%error, "the service already exists; updating its configuration");
            let service = manager
                .open_service(SERVICE_NAME, access)
                .context("cannot open the service; run as an administrator")?;
            // A reinstall points the service at the new executable path.
            service
                .change_config(&info)
                .context("cannot update the service configuration")?;
            service
        }
    };
    service.set_description("The ResticHub backup agent")?;
    // The service restarts after a failure; a repeated failure stays visible in the event log.
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(30),
            },
        ]),
    })?;
    service.set_failure_actions_on_non_crash_failures(true)?;
    if service.query_status()?.current_state != ServiceState::Running {
        // A start failure is reported, not hidden behind a success message.
        service
            .start(&[] as &[&OsStr])
            .context("cannot start the service")?;
    }
    println!("the service {SERVICE_NAME} is installed and started");
    Ok(())
}

/// Stops the Windows service, waits for it and starts it again, when it is installed.
///
/// # Errors
///
/// Returns an error when the service exists and cannot be stopped or started.
pub fn restart_if_installed() -> Result<bool> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("cannot open the service manager; run as an administrator")?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };
    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop().context("cannot stop the service")?;
        let deadline = std::time::Instant::now() + STOP_WAIT;
        while service.query_status()?.current_state != ServiceState::Stopped {
            if std::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "the service {SERVICE_NAME} did not stop in time; start it by hand after it stops"
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    service
        .start(&[] as &[&OsStr])
        .context("cannot start the service")?;
    println!("the service {SERVICE_NAME} is restarted");
    Ok(true)
}

/// Stops and removes the Windows service (FR-IN-08).
///
/// # Errors
///
/// Returns an error when the service cannot be removed, usually without administrator rights.
pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("cannot open the service manager; run as an administrator")?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) =>
        {
            println!("the service {SERVICE_NAME} is not installed");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    service.delete()?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        let _ = service.stop();
    }
    println!("the service {SERVICE_NAME} is removed");
    Ok(())
}
