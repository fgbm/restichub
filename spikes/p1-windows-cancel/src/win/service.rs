//! Windows service glue: the service entry point and the install commands.

use std::env;
use std::ffi::OsString;
use std::time::Duration;

use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{Result as ServiceResult, define_windows_service, service_dispatcher};

use super::Config;

/// The service name. It must match the installed service.
pub const SERVICE_NAME: &str = "P1CancelSpike";

define_windows_service!(ffi_service_main, service_main);

/// Start the service dispatcher. This call blocks until the service stops.
pub fn run_as_service() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|error| error.to_string())
}

fn service_main(_arguments: Vec<OsString>) {
    match Config::from_env_run_args() {
        Ok(config) => {
            if let Err(error) = run_service(&config) {
                eprintln!("p1-cancel service error: {error}");
            }
        }
        Err(error) => eprintln!("p1-cancel service argument error: {error}"),
    }
}

fn run_service(config: &Config) -> ServiceResult<()> {
    let event_handler = |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Interrogate => {
            ServiceControlHandlerResult::NoError
        }
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
    status_handle.set_service_status(service_status(ServiceState::Running))?;

    if let Err(error) = super::run_experiment_and_write(config) {
        eprintln!("p1-cancel experiment error: {error}");
    }

    status_handle.set_service_status(service_status(ServiceState::Stopped))?;
    Ok(())
}

fn service_status(state: ServiceState) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

/// Install the service under the LocalSystem account.
pub fn install(config: &Config) -> Result<(), String> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|error| format!("ServiceManager: {error}"))?;

    let executable = env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("P1 cancel spike"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: executable,
        launch_arguments: config.service_launch_arguments(),
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };

    let service = manager
        .create_service(&info, ServiceAccess::QUERY_STATUS)
        .map_err(|error| format!("create_service: {error}"))?;
    drop(service);
    Ok(())
}

/// Stop and delete the service.
pub fn uninstall() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|error| format!("ServiceManager: {error}"))?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::DELETE)
        .map_err(|error| format!("open_service: {error}"))?;
    let _ = service.stop();
    service
        .delete()
        .map_err(|error| format!("delete: {error}"))?;
    Ok(())
}
