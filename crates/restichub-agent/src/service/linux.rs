//! The systemd unit of the agent (FR-AG-01, FR-IN-08, AC-IN-03).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const SERVICE_NAME: &str = "restichub-agent";

/// The writable cache directory that systemd creates for the unit.
pub const CACHE_DIR: &str = "/var/cache/restichub-agent";

/// The capabilities restic needs on a backup source and on a local repository.
///
/// Restic runs as `root`, so the capability bounding set is the only limit. It drops every other
/// capability. Without these five, restic cannot read a file of another account (CAP_DAC_READ_SEARCH,
/// CAP_DAC_OVERRIDE) or keep an ownership and a mode of a restored file (CAP_CHOWN, CAP_FOWNER,
/// CAP_FSETID).
const CAPABILITIES: &str = "CAP_DAC_READ_SEARCH CAP_DAC_OVERRIDE CAP_CHOWN CAP_FOWNER CAP_FSETID";

fn unit_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system").join(format!("{SERVICE_NAME}.service"))
}

/// The systemd unit text for the running agent (FR-AG-01, AC-IN-03).
///
/// `ProtectSystem=strict` makes the whole file system read-only except the identity directory, the
/// cache directory and the restore roots (FR-RS-03). A read of a backup source stays possible. A
/// write to a local repository fails, so the operator adds every local repository path to
/// `ReadWritePaths`. `ProtectHome` stays off, because a backup source is often below `/home`.
#[must_use]
pub fn unit_contents(exe: &Path, config_dir: &Path, restore_roots: &[PathBuf]) -> String {
    let restore_paths: String = restore_roots
        .iter()
        .map(|root| format!(" \"{}\"", root.display()))
        .collect();
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
         CapabilityBoundingSet={capabilities}\n\
         RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\n\
         CacheDirectory=restichub-agent\n\
         Environment=RESTIC_CACHE_DIR={cache}\n\
         ReadWritePaths=\"{config}\"{restore_paths}\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        exe = exe.display(),
        capabilities = CAPABILITIES,
        cache = CACHE_DIR,
        config = config_dir.display(),
        restore_paths = restore_paths,
    )
}

/// Writes and enables the systemd unit (FR-IN-08, AC-IN-03).
///
/// # Errors
///
/// Returns an error when the unit cannot be written or `systemctl` fails.
pub fn install() -> Result<()> {
    let exe = std::env::current_exe().context("cannot find the agent executable")?;
    // A binary that another account can replace defeats the service hardening (T20).
    super::location::check(&exe)?;
    let config = crate::paths::config_dir();
    // A path in `ReadWritePaths` must exist, or systemd refuses to start the unit. The default root
    // is created here with the access of root only; an extra root is the choice of the operator.
    crate::restore_roots::ensure_default_root()?;
    let access = crate::restore_roots::RestoreAccess::for_host(&config, Path::new(CACHE_DIR), &exe);
    for root in access.roots() {
        if !root.is_dir() {
            bail!(
                "the restore root {} does not exist; create it and run install-service again",
                root.display()
            );
        }
    }
    let path = unit_path();
    std::fs::write(&path, unit_contents(&exe, &config, access.roots()))
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
            Path::new("/opt/restichub-agent/restichub-agent"),
            Path::new("/etc/restichub-agent"),
            &[PathBuf::from("/var/lib/restichub-restore")],
        );
        assert!(unit.contains("ExecStart=\"/opt/restichub-agent/restichub-agent\" run"));
        assert!(
            unit.contains(
                "ReadWritePaths=\"/etc/restichub-agent\" \"/var/lib/restichub-restore\"\n"
            )
        );
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("PrivateTmp=true"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn ac_ag_07_the_unit_stops_the_whole_control_group() {
        let unit = unit_contents(
            Path::new("/opt/restichub-agent/restichub-agent"),
            Path::new("/etc/restichub-agent"),
            &[PathBuf::from("/var/lib/restichub-restore")],
        );
        // The default `KillMode=control-group` also stops a restic tree after an agent stop.
        assert!(
            !unit.contains("KillMode="),
            "a KillMode other than the default can leave a restic child"
        );
    }

    #[test]
    fn ac_in_03_the_unit_keeps_the_restic_capabilities() {
        let unit = unit_contents(
            Path::new("/opt/restichub-agent/restichub-agent"),
            Path::new("/etc/restichub-agent"),
            &[PathBuf::from("/var/lib/restichub-restore")],
        );
        // A backup source below /home must stay readable, so ProtectHome must be absent.
        assert!(
            !unit.contains("ProtectHome"),
            "ProtectHome breaks a source under /home"
        );
        for capability in CAPABILITIES.split_whitespace() {
            assert!(
                unit.contains(capability),
                "the bounding set drops {capability}"
            );
        }
        assert!(unit.contains("CacheDirectory=restichub-agent"));
        assert!(unit.contains("RESTIC_CACHE_DIR=/var/cache/restichub-agent"));
    }

    #[test]
    fn fr_rs_03_the_unit_can_write_every_restore_root() {
        let unit = unit_contents(
            Path::new("/opt/restichub-agent/restichub-agent"),
            Path::new("/etc/restichub-agent"),
            &[
                PathBuf::from("/var/lib/restichub-restore"),
                PathBuf::from("/srv/restore"),
            ],
        );
        assert!(unit.contains("ReadWritePaths=\"/etc/restichub-agent\" \"/var/lib/restichub-restore\" \"/srv/restore\"\n"));
    }
}
