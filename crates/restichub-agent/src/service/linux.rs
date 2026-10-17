//! The systemd unit of the agent (FR-AG-01, FR-IN-08, AC-IN-03).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use restichub_proto::restore_path::{PathFlavor, components, overlaps};

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
/// `ProtectSystem=strict` and `ProtectHome=read-only` make the whole file system read-only except
/// the identity directory, the cache directory and `writable_roots`: the restore roots (FR-RS-03)
/// and the repository roots (ADR-0045). `ProtectSystem=strict` alone leaves `/home`, `/root` and
/// `/run/user` writable. A read of a backup source stays possible.
#[must_use]
pub fn unit_contents(exe: &Path, config_dir: &Path, writable_roots: &[PathBuf]) -> String {
    let writable_paths: String = writable_roots
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
         RestartPreventExitStatus={no_identity}\n\
         UMask=0077\n\
         LimitCORE=0\n\
         NoNewPrivileges=true\n\
         ProtectSystem=strict\n\
         ProtectHome=read-only\n\
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
         ReadWritePaths=\"{config}\"{writable_paths}\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        exe = exe.display(),
        no_identity = crate::commands::EXIT_NO_IDENTITY,
        capabilities = CAPABILITIES,
        cache = CACHE_DIR,
        config = config_dir.display(),
        writable_paths = writable_paths,
    )
}

/// The roots that the installed unit cannot write, because the operator added them after
/// `install-service` (FR-RS-03, ADR-0045). Restic fails with `EROFS` on such a root.
#[must_use]
pub fn roots_missing_from_installed_unit(roots: &[PathBuf]) -> Vec<PathBuf> {
    match std::fs::read_to_string(unit_path()) {
        Ok(unit) => roots_missing_from_unit(&unit, roots),
        Err(_) => Vec::new(),
    }
}

fn roots_missing_from_unit(unit: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let writable: Vec<&str> = unit
        .lines()
        .filter_map(|line| line.strip_prefix("ReadWritePaths="))
        .flat_map(|paths| paths.split('"').skip(1).step_by(2))
        .collect();
    roots
        .iter()
        .filter(|root| {
            !writable
                .iter()
                .any(|allowed| root.starts_with(Path::new(allowed)))
        })
        .cloned()
        .collect()
}

/// Writes and enables the systemd unit (FR-IN-08, AC-IN-03).
///
/// # Errors
///
/// Returns an error when the unit cannot be written or `systemctl` fails.
pub fn install(restic: &Path) -> Result<()> {
    let exe = crate::paths::agent_exe();
    // A binary that another account can replace defeats the service hardening (T20).
    super::location::check(&exe, restic)?;
    let config = crate::paths::config_dir();
    // A path in `ReadWritePaths` must exist, or systemd refuses to start the unit. The default root
    // is created here with the access of root only; an extra root is the choice of the operator.
    crate::restore_roots::ensure_default_root()?;
    refuse_rejected_roots(&config)?;
    let access =
        crate::restore_roots::RestoreAccess::for_host(&config, Path::new(CACHE_DIR), restic);
    for root in access.roots() {
        if !root.is_dir() {
            bail!(
                "the restore root {} does not exist; create it and run install-service again",
                root.display()
            );
        }
    }
    let repository_roots = repository_roots(&config, restic)?;
    if let Some((restore_root, repository_root)) =
        overlapping_roots(access.roots(), &repository_roots)
    {
        bail!(
            "the restore root {} and the repository root {} overlap, so a restore could overwrite a repository; remove one of them from its roots file and run install-service again",
            restore_root.display(),
            repository_root.display()
        );
    }
    let writable: Vec<PathBuf> = access
        .roots()
        .iter()
        .chain(&repository_roots)
        .cloned()
        .collect();
    let path = unit_path();
    std::fs::write(&path, unit_contents(&exe, &config, &writable))
        .with_context(|| format!("cannot write {}", path.display()))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", SERVICE_NAME])?;
    // A restart also starts a stopped unit, and it makes a running unit use the new unit file.
    systemctl(&["restart", SERVICE_NAME])?;
    Ok(())
}

/// Reads `systemctl` output; a missing unit is a state, not an error.
///
/// # Errors
///
/// Returns an error when the `systemctl` program does not start, for example when it is not installed.
pub fn state() -> Result<super::ServiceState> {
    use super::{ServiceState, StartType};
    if !unit_path().exists() {
        return Ok(ServiceState::NOT_INSTALLED);
    }
    let active = systemctl_text(&["is-active", SERVICE_NAME])?;
    let enabled = systemctl_text(&["is-enabled", SERVICE_NAME])?;
    Ok(ServiceState {
        installed: true,
        running: active == "active",
        start_type: match enabled.as_str() {
            "enabled" | "enabled-runtime" => StartType::Automatic,
            "disabled" => StartType::Disabled,
            "static" | "indirect" => StartType::Manual,
            _ => StartType::Unknown,
        },
    })
}

/// Stops the active unit and leaves an absent or stopped one alone.
///
/// # Errors
///
/// Returns an error when `systemctl` cannot stop the unit.
pub fn stop() -> Result<()> {
    if unit_path().exists() && systemctl_text(&["is-active", SERVICE_NAME])? == "active" {
        systemctl(&["stop", SERVICE_NAME])?;
    }
    Ok(())
}

/// Refuses a restore root that another account can change (FR-RS-03).
fn refuse_rejected_roots(config: &Path) -> Result<()> {
    if let Some((root, reason)) = crate::restore_roots::rejected_roots(config)
        .into_iter()
        .next()
    {
        bail!("the restore root {} is not safe: {reason}", root.display());
    }
    Ok(())
}

/// The repository roots of the host, each created with the access of root only when it is missing
/// (ADR-0045). A root that another account can change, a system directory and a root on the files
/// of the agent stop the install.
fn repository_roots(config: &Path, restic: &Path) -> Result<Vec<PathBuf>> {
    let file = crate::repository_roots::roots_file_for(config);
    if let Some((root, reason)) = crate::restore_roots::rejected_roots_in(&file)
        .into_iter()
        .next()
    {
        bail!(
            "the repository root {} is not safe: {reason}",
            root.display()
        );
    }
    let roots = crate::restore_roots::load_roots_file(&file);
    let protected = crate::restore_roots::protected_paths(config, Path::new(CACHE_DIR), restic);
    for root in &roots {
        if let Some(reason) = crate::repository_roots::refused_root(root, &protected) {
            bail!(
                "the repository root {} in {} is not allowed: {reason}; remove the line and run install-service again",
                root.display(),
                file.display()
            );
        }
    }
    for root in &roots {
        if !root.is_dir() {
            create_repository_root(root)?;
        }
    }
    Ok(roots)
}

/// Missing parents get mode `0755` whatever the umask, so the root check accepts them later.
fn create_repository_root(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    if let Some(parent) = root.parent() {
        crate::paths::ensure_program_dir(parent)?;
    }
    std::fs::create_dir(root)
        .with_context(|| format!("cannot create the repository root {}", root.display()))?;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("cannot set the mode of {}", root.display()))
}

fn is_filesystem_root(path: &Path) -> bool {
    path.to_str()
        .and_then(|path| components(PathFlavor::host(), path))
        .is_some_and(|parts| parts.is_empty())
}

/// The first restore root and repository root where one holds the other. A restore there could
/// overwrite a repository. The restore root `/` is the documented root of an in-place restore, so
/// it stays allowed; the restore check keeps it out of the repository roots.
fn overlapping_roots(restore: &[PathBuf], repository: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    let flavor = PathFlavor::host();
    let forms = |path: &Path| -> Vec<String> {
        let mut forms: Vec<String> = path.to_str().map(str::to_owned).into_iter().collect();
        forms.extend(
            path.canonicalize()
                .ok()
                .and_then(|real| real.to_str().map(str::to_owned)),
        );
        forms
    };
    restore
        .iter()
        .filter(|root| !is_filesystem_root(root))
        .find_map(|restore_root| {
            let restore_forms = forms(restore_root);
            repository.iter().find_map(|repository_root| {
                let hit = forms(repository_root).iter().any(|repository_form| {
                    restore_forms
                        .iter()
                        .any(|restore_form| overlaps(flavor, restore_form, repository_form))
                });
                hit.then(|| (restore_root.clone(), repository_root.clone()))
            })
        })
}

/// Restarts the unit when it is installed.
///
/// # Errors
///
/// Returns an error when `systemctl` cannot restart the unit.
pub fn restart_if_installed() -> Result<bool> {
    if !unit_path().exists() {
        return Ok(false);
    }
    systemctl(&["restart", SERVICE_NAME])?;
    println!("the service {SERVICE_NAME} is restarted");
    Ok(true)
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

/// The first output line of a query. `is-active` and `is-enabled` exit non-zero for a plain "no".
fn systemctl_text(args: &[&str]) -> Result<String> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .context("cannot run systemctl")?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned())
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
        assert!(unit.contains("RestartPreventExitStatus=78\n"));
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
        // A backup source below /home stays readable; only a write needs a root.
        assert!(unit.contains("ProtectHome=read-only\n"));
        assert!(!unit.contains("ProtectHome=true") && !unit.contains("ProtectHome=tmpfs"));
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

    #[test]
    fn adr_0045_the_unit_can_write_every_repository_root() {
        let unit = unit_contents(
            Path::new("/opt/restichub-agent/restichub-agent"),
            Path::new("/etc/restichub-agent"),
            &[
                PathBuf::from("/var/lib/restichub-restore"),
                PathBuf::from("/var/spool/restichub"),
            ],
        );
        assert!(unit.contains("ReadWritePaths=\"/etc/restichub-agent\" \"/var/lib/restichub-restore\" \"/var/spool/restichub\"\n"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert_eq!(
            roots_missing_from_unit(&unit, &[PathBuf::from("/var/spool/restichub/local-test")]),
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            roots_missing_from_unit(&unit, &[PathBuf::from("/srv/repos")]),
            vec![PathBuf::from("/srv/repos")]
        );
    }

    #[test]
    fn fr_rs_03_a_root_added_after_install_is_reported_as_read_only() {
        let unit = unit_contents(
            Path::new("/opt/restichub-agent/restichub-agent"),
            Path::new("/etc/restichub-agent"),
            &[PathBuf::from("/var/lib/restichub-restore")],
        );
        let roots = [
            PathBuf::from("/var/lib/restichub-restore"),
            PathBuf::from("/var/lib/restichub-restore/nested"),
            PathBuf::from("/srv/restore"),
        ];
        assert_eq!(
            roots_missing_from_unit(&unit, &roots),
            vec![PathBuf::from("/srv/restore")]
        );
        assert!(roots_missing_from_unit(&unit, &roots[..2]).is_empty());
    }

    #[test]
    fn adr_0045_a_restore_root_that_overlaps_a_repository_root_is_refused() {
        let repository = [PathBuf::from("/srv/repos")];
        for restore in ["/srv", "/srv/repos", "/srv/repos/fs-01", "/srv//repos/"] {
            assert_eq!(
                overlapping_roots(&[PathBuf::from(restore)], &repository),
                Some((PathBuf::from(restore), PathBuf::from("/srv/repos"))),
                "{restore}"
            );
        }
        let apart = [
            PathBuf::from("/var/lib/restichub-restore"),
            PathBuf::from("/srv/restore"),
            PathBuf::from("/srv/repos2"),
        ];
        assert_eq!(overlapping_roots(&apart, &repository), None);
    }

    #[test]
    fn adr_0045_the_restore_root_slash_stays_allowed_next_to_a_repository_root() {
        let restore = [
            PathBuf::from("/"),
            PathBuf::from("/var/lib/restichub-restore"),
        ];
        assert_eq!(
            overlapping_roots(&restore, &[PathBuf::from("/srv/repos")]),
            None
        );
    }
}
