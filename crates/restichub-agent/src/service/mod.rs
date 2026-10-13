//! The operating system service of the agent (FR-AG-01, FR-IN-08).

use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::run_as_service;

/// Writes and enables the operating system service (FR-IN-08).
///
/// # Errors
///
/// Returns an error when the service cannot be written or enabled, or when the operating system is
/// not supported in the MVP.
pub fn install() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::install()
    }
    #[cfg(windows)]
    {
        windows::install()
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
