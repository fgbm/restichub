//! The Windows service of the agent (FR-AG-01, FR-IN-08, AC-IN-03).

use std::ffi::{OsStr, OsString};

use anyhow::{Context, Result};
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

pub const SERVICE_NAME: &str = "restichub-agent";

/// The Win32 error `ERROR_SERVICE_DOES_NOT_EXIST`, which means the service is not installed.
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

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
    status_handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP))?;
    let result = crate::commands::run(&crate::paths::config_dir(), Some(receiver));
    status_handle
        .set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    result
}

fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: std::time::Duration::default(),
        process_id: None,
    }
}

/// Writes and starts the Windows service (FR-IN-08, AC-IN-03).
///
/// An existing service is updated instead of created, so the command is idempotent.
///
/// # Errors
///
/// Returns an error when the service cannot be created or started, usually without administrator
/// rights.
pub fn install() -> Result<()> {
    let exe = std::env::current_exe().context("cannot find the agent executable")?;
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
            tracing::debug!(%error, "the service already exists; updating it");
            manager
                .open_service(SERVICE_NAME, access)
                .context("cannot create or open the service; run as an administrator")?
        }
    };
    service.set_description("The ResticHub backup agent")?;
    if service.query_status()?.current_state != ServiceState::Running {
        let _ = service.start(&[] as &[&OsStr]);
    }
    println!("the service {SERVICE_NAME} is installed and started");
    Ok(())
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
