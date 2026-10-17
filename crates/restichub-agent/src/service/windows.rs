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

/// A running job gets its stop grace first, so this bound is generous.
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
    // The identity load puts back an interrupted swap, after the identity check of `run`.
    crate::logging::init_service();
    if let Err(error) = run_service() {
        tracing::error!(%error, "the service stopped with an error");
        // Without a stopped status the service manager counts a failure and restarts the service,
        // as `Restart=on-failure` does on Linux.
        std::process::exit(1);
    }
}

/// Returns an error only for a failure that a restart can fix. A refusal that needs an
/// administrator reports the stopped state with the exit code 78, which the service manager does
/// not act on, as `RestartPreventExitStatus=78` on Linux.
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
    let exit_code = match crate::commands::run(&crate::paths::config_dir(), Some(receiver)) {
        Ok(()) => ServiceExitCode::NO_ERROR,
        // `run` has logged the refusal with its fix.
        Err(error) if crate::commands::needs_administrator(&error) => {
            ServiceExitCode::ServiceSpecific(u32::from(crate::commands::EXIT_NO_IDENTITY))
        }
        Err(error) => return Err(error),
    };
    status_handle.set_service_status(status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit_code,
    ))?;
    Ok(())
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
    let exe = crate::paths::agent_exe();
    // A binary that another account can replace defeats the service hardening (T20).
    super::location::check(&exe, restic)?;
    // The default restore root gets the access of SYSTEM and Administrators only (FR-RS-03).
    crate::restore_roots::ensure_default_root()?;
    crate::restore_roots::refuse_skipped_lines(&crate::restore_roots::roots_file())?;
    if let Some((root, reason)) = crate::restore_roots::rejected_roots(&crate::paths::config_dir())
        .into_iter()
        .next()
    {
        anyhow::bail!("the restore root {} is not safe: {reason}", root.display());
    }
    // Before the service changes, so a registry failure leaves the service as it was.
    crate::logging::register_event_source()?;
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("cannot open the service manager; run as an administrator")?;
    // Until a clean run after a restart, a process that kept the program directory open during a
    // repair in place can still add a library there, so the boot must not start the service (T20).
    let start_type = if crate::program_repair::manual_start()? {
        ServiceStartType::OnDemand
    } else {
        ServiceStartType::AutoStart
    };
    let info = service_info(start_type);
    // STOP is needed because a reinstall restarts a running service.
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP;
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
    service.update_failure_actions(failure_actions())?;
    // A stop with the exit code 78 is not a crash, so the actions skip it.
    service.set_failure_actions_on_non_crash_failures(false)?;
    // A running service restarts, so that it uses the new executable path.
    stop_and_wait(&service)?;
    // A start failure is reported, not hidden behind a success message.
    service
        .start(&[] as &[&OsStr])
        .context("cannot start the service")?;
    Ok(())
}

fn service_info(start_type: ServiceStartType) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("ResticHub Agent"),
        service_type: ServiceType::OWN_PROCESS,
        start_type,
        error_control: ServiceErrorControl::Normal,
        executable_path: crate::paths::agent_exe(),
        launch_arguments: vec![OsString::from("run")],
        dependencies: vec![],
        // A missing account runs the service as LocalSystem (FR-AG-01).
        account_name: None,
        account_password: None,
    }
}

/// Lets an installed service start only by hand; an absent service is not an error.
///
/// # Errors
///
/// Returns an error when the service exists and its configuration cannot be changed.
pub fn set_manual_start() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("cannot open the service manager; run as an administrator")?;
    match manager.open_service(SERVICE_NAME, ServiceAccess::CHANGE_CONFIG) {
        Ok(service) => service
            .change_config(&service_info(ServiceStartType::OnDemand))
            .context("cannot set the manual start of the service"),
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// The service restarts after each failure. The service control manager repeats the last action for
/// each later failure, so the restarts never end. The count resets after a day without a failure.
fn failure_actions() -> ServiceFailureActions {
    let restart = |seconds| ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(seconds),
    };
    ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(5), restart(30), restart(60)]),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopStep {
    Done,
    Wait,
    Stop,
}

/// A service that starts or stops already accepts no stop control, so the command waits for it.
fn stop_step(state: ServiceState) -> StopStep {
    match state {
        ServiceState::Stopped => StopStep::Done,
        ServiceState::StartPending | ServiceState::StopPending => StopStep::Wait,
        _ => StopStep::Stop,
    }
}

fn stop_and_wait(service: &windows_service::service::Service) -> Result<()> {
    let deadline = std::time::Instant::now() + STOP_WAIT;
    // The process reports the stopped state before it exits, and until then its image and its
    // restic keep files of the program directory open. The handle is opened before the stop, so a
    // reused process id cannot take its place.
    let mut process = None;
    let mut sent = false;
    loop {
        let status = service.query_status()?;
        let step = stop_step(status.current_state);
        if step == StopStep::Done {
            break;
        }
        if process.is_none() {
            process = status.process_id.and_then(open_process);
        }
        if step == StopStep::Stop
            && !sent
            && let Err(error) = service.stop()
        {
            // The state can change between the query and the control.
            if stop_step(service.query_status()?.current_state) == StopStep::Stop {
                return Err(error).context("cannot stop the service");
            }
        }
        sent |= step == StopStep::Stop;
        if std::time::Instant::now() >= deadline {
            return Err(late_stop());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if let Some(process) = process
        && !wait_for_exit(
            &process,
            deadline.saturating_duration_since(std::time::Instant::now()),
        )
    {
        return Err(late_stop());
    }
    Ok(())
}

/// The tree stays as it was, so the advice is a new run and never a start from it.
fn late_stop() -> anyhow::Error {
    anyhow::anyhow!(
        "the service {SERVICE_NAME} did not stop in {} seconds; wait until `Get-Service {SERVICE_NAME}` shows Stopped, then run the command again",
        STOP_WAIT.as_secs()
    )
}

#[allow(unsafe_code)]
fn open_process(pid: u32) -> Option<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::FromRawHandle as _;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
    // SAFETY: the call takes no pointer; a null result means no handle.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    // SAFETY: a non-null handle is open and owned by nothing else.
    (!handle.is_null())
        .then(|| unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) })
}

#[allow(unsafe_code)]
fn wait_for_exit(process: &std::os::windows::io::OwnedHandle, timeout: Duration) -> bool {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    // SAFETY: the handle is open with SYNCHRONIZE for the call.
    unsafe { WaitForSingleObject(process.as_raw_handle(), millis) == WAIT_OBJECT_0 }
}

/// # Errors
///
/// Returns an error when the manager cannot be opened or queried.
pub fn state() -> Result<super::ServiceState> {
    use super::{ServiceState as Reported, StartType};
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("cannot open the service manager")?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) =>
        {
            return Ok(Reported::NOT_INSTALLED);
        }
        Err(error) => return Err(error.into()),
    };
    let running = service.query_status()?.current_state == ServiceState::Running;
    let start_type = match service.query_config()?.start_type {
        ServiceStartType::AutoStart => StartType::Automatic,
        ServiceStartType::OnDemand => StartType::Manual,
        ServiceStartType::Disabled => StartType::Disabled,
        _ => StartType::Unknown,
    };
    Ok(Reported {
        installed: true,
        running,
        start_type,
    })
}

/// # Errors
///
/// Returns an error when the service exists and does not stop.
pub fn stop() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("cannot open the service manager; run as an administrator")?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP;
    match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => stop_and_wait(&service),
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
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
    stop_and_wait(&service)?;
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
            crate::program_repair::forget()?;
            return crate::logging::remove_event_source();
        }
        Err(error) => return Err(error.into()),
    };
    service.delete()?;
    // The stopping service still writes events, so the source goes only after it stopped.
    stop_and_wait(&service)?;
    crate::logging::remove_event_source()?;
    crate::program_repair::forget()?;
    println!("the service {SERVICE_NAME} is removed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_in_07_a_service_that_starts_or_stops_is_waited_for_and_not_sent_a_stop() {
        assert_eq!(stop_step(ServiceState::Stopped), StopStep::Done);
        assert_eq!(stop_step(ServiceState::StopPending), StopStep::Wait);
        assert_eq!(stop_step(ServiceState::StartPending), StopStep::Wait);
        for state in [
            ServiceState::Running,
            ServiceState::Paused,
            ServiceState::PausePending,
            ServiceState::ContinuePending,
        ] {
            assert_eq!(stop_step(state), StopStep::Stop, "{state:?}");
        }
        let advice = late_stop().to_string();
        assert!(advice.contains("run the command again"), "{advice}");
        assert!(!advice.contains("start it"), "{advice}");
    }

    #[test]
    fn fr_ag_01_every_failure_restarts_the_service() {
        let actions = failure_actions().actions.unwrap_or_default();
        let steps: Vec<(ServiceActionType, u64)> = actions
            .iter()
            .map(|action| (action.action_type, action.delay.as_secs()))
            .collect();
        assert_eq!(
            steps,
            [
                (ServiceActionType::Restart, 5),
                (ServiceActionType::Restart, 30),
                (ServiceActionType::Restart, 60)
            ]
        );
    }
}
