//! The operating system service of the agent (FR-AG-01, FR-IN-08).

use std::path::Path;

use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

mod location;

#[cfg(windows)]
pub use windows::run_as_service;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartType {
    Automatic,
    Manual,
    Disabled,
    Unknown,
}

impl std::fmt::Display for StartType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Automatic => "automatic",
            Self::Manual => "manual",
            Self::Disabled => "disabled",
            Self::Unknown => "unknown",
        })
    }
}

/// What the operating system says about the agent service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceState {
    pub installed: bool,
    pub running: bool,
    pub start_type: StartType,
}

impl ServiceState {
    pub const NOT_INSTALLED: Self = Self {
        installed: false,
        running: false,
        start_type: StartType::Unknown,
    };
}

/// The operating system service manager (systemd or the Windows service control manager).
///
/// A test replaces it, so the setup logic runs without a real manager (FR-IN-08).
pub trait ServiceManager {
    /// Writes or updates the service for the running executable, enables it and (re)starts it.
    /// A second call updates the executable path and restarts the service.
    ///
    /// # Errors
    ///
    /// Returns an error when the service cannot be written, enabled or started.
    fn install_and_start(&self, restic: &Path) -> Result<()>;

    /// Reports installed, running and start type; a missing service is not an error.
    ///
    /// # Errors
    ///
    /// Returns an error when the manager cannot be asked.
    fn state(&self) -> Result<ServiceState>;

    /// Stops the running service and waits; an absent or stopped service is not an error.
    ///
    /// # Errors
    ///
    /// Returns an error when the running service does not stop.
    fn stop(&self) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HostService;

impl ServiceManager for HostService {
    fn install_and_start(&self, restic: &Path) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            linux::install(restic)
        }
        #[cfg(windows)]
        {
            windows::install(restic)
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = restic;
            anyhow::bail!("the service install is a 1.0 feature on this operating system")
        }
    }

    fn stop(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            linux::stop()
        }
        #[cfg(windows)]
        {
            windows::stop()
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Ok(())
        }
    }

    fn state(&self) -> Result<ServiceState> {
        #[cfg(target_os = "linux")]
        {
            linux::state()
        }
        #[cfg(windows)]
        {
            windows::state()
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Ok(ServiceState::NOT_INSTALLED)
        }
    }
}

/// Refuses an agent or restic location that another account can change (T20, FR-IN-08).
///
/// # Errors
///
/// Returns an error when the executable, restic or a parent directory is writable by another
/// account.
pub fn check_install_location(restic: &Path) -> Result<()> {
    location::check(&crate::paths::agent_exe(), restic)
}

/// Stops and removes the operating system service (FR-IN-08).
///
/// # Errors
///
/// Returns an error when the service cannot be removed, or when the operating system is not
/// supported in the MVP.
pub fn uninstall() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::uninstall()
    }
    #[cfg(windows)]
    {
        windows::uninstall()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        anyhow::bail!("the service install is a 1.0 feature on this operating system")
    }
}

/// Restarts the installed service so that it runs with the new restic, and tells whether a
/// service was installed (ADR-0038).
///
/// # Errors
///
/// Returns an error when the service exists and cannot be restarted.
pub fn restart_if_installed() -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        linux::restart_if_installed()
    }
    #[cfg(windows)]
    {
        windows::restart_if_installed()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Ok(false)
    }
}
