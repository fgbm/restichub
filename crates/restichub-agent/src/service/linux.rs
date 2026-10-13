//! The systemd unit of the agent (FR-AG-01, FR-IN-08, AC-IN-03).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const SERVICE_NAME: &str = "restichub-agent";

fn unit_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system").join(format!("{SERVICE_NAME}.service"))
}

/// The systemd unit text for the running agent (FR-AG-01, AC-IN-03).
///
/// `ProtectSystem=strict` makes the whole file system read-only except the identity directory.
/// Restic reads the backup sources, which the read-only view permits, but it cannot write to a
/// local repository. The operator adds every local repository path to `ReadWritePaths`.
#[must_use]
pub fn unit_contents(exe: &Path, config_dir: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=ResticHub agent\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart=\"{exe}\" run\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         UMask=0077\n\
         LimitCORE=0\n\
         NoNewPrivileges=true\n\
         ProtectSystem=strict\n\
         ProtectHome=true\n\
         PrivateTmp=true\n\
         PrivateDevices=true\n\
         ProtectKernelTunables=true\n\
         ProtectKernelModules=true\n\
         ProtectKernelLogs=true\n\
         ProtectControlGroups=true\n\
         ProtectClock=true\n\
         ProtectHostname=true\n\
         RestrictNamespaces=true\n\
         RestrictRealtime=true\n\
         RestrictSUIDSGID=true\n\
         LockPersonality=true\n\
         MemoryDenyWriteExecute=true\n\
         SystemCallArchitectures=native\n\
         CapabilityBoundingSet=\n\
         AmbientCapabilities=\n\
         RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\n\
         ReadWritePaths=\"{config}\"\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        exe = exe.display(),
        config = config_dir.display(),
    )
}

/// Writes and enables the systemd unit (FR-IN-08, AC-IN-03).
///
/// # Errors
///
/// Returns an error when the unit cannot be written or `systemctl` fails.
pub fn install() -> Result<()> {
    let exe = std::env::current_exe().context("cannot find the agent executable")?;
    let config = crate::paths::config_dir();
    let path = unit_path();
    std::fs::write(&path, unit_contents(&exe, &config))
        .with_context(|| format!("cannot write {}", path.display()))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", SERVICE_NAME])?;
    println!("the unit {} is installed and enabled", path.display());
    Ok(())
}

/// Stops and removes the systemd unit (FR-IN-08).
///
/// # Errors
///
/// Returns an error when the unit cannot be removed or `systemctl` fails.
pub fn uninstall() -> Result<()> {
    let path = unit_path();
    if !path.exists() {
        println!("the unit {} is not installed", path.display());
        return Ok(());
    }
    // A unit that is not running or not enabled still needs its file removed.
    let _ = systemctl(&["disable", "--now", SERVICE_NAME]);
    std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    systemctl(&["daemon-reload"])?;
    println!("the unit {} is removed", path.display());
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .context("cannot run systemctl")?;
    if !status.success() {
        bail!("systemctl {} failed with {status}", args.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn ac_in_03_the_unit_runs_the_agent_and_hardens_it() {
        let unit = unit_contents(
            Path::new("/usr/local/bin/restichub-agent"),
            Path::new("/etc/restichub-agent"),
        );
        assert!(unit.contains("ExecStart=\"/usr/local/bin/restichub-agent\" run"));
        assert!(unit.contains("ReadWritePaths=\"/etc/restichub-agent\""));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("PrivateTmp=true"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }
}
