//! The operating system service of the agent (FR-AG-01, FR-IN-08).

use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

mod location;

#[cfg(windows)]
pub use windows::run_as_service;

/// Writes and enables the operating system service (FR-IN-08).
///
/// # Errors
///
/// Returns an error when the service cannot be written or enabled, when restic cannot be installed,
/// or when the operating system is not supported in the MVP.
pub fn install(restic: &crate::cli::ResticFileOptions) -> Result<()> {
    crate::preflight::ensure_elevated()?;
    crate::preflight::prepare_directories(&crate::paths::config_dir(), &crate::paths::cache_dir())?;
    let restic = crate::commands::ensure_restic(restic)?;
    #[cfg(target_os = "linux")]
    {
        linux::install(restic.path())
    }
    #[cfg(windows)]
    {
        windows::install(restic.path())
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        anyhow::bail!("the service install is a 1.0 feature on this operating system")
    }
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
