//! Where the agent may restore files (FR-RS-03, ADR-0035).
//!
//! The agent restores as root or LocalSystem, so a restore target decides which file the agent can
//! overwrite. Only root or Administrators configure the roots, in a protected file like the trust
//! mode file (ADR-0037). A job and the control plane cannot change them.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use restichub_proto::restore_path::{PathFlavor, components, is_within, overlaps};

/// The default restore root: the only root when the roots file is absent.
#[must_use]
pub fn default_root() -> PathBuf {
    #[cfg(windows)]
    {
        let program_data = std::env::var_os("ProgramData")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
        // Outside the agent tree, which the agent protects with its own DACL (ADR-0035).
        program_data.join("ResticHub-Restore")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/restichub-restore")
    }
}

/// The file with the extra restore roots, one absolute path per line. It lies next to the
/// configuration directory, so a swap of the identity never removes it.
#[must_use]
pub fn roots_file() -> PathBuf {
    roots_file_for(&crate::paths::config_dir())
}

pub(crate) fn roots_file_for(config_dir: &Path) -> PathBuf {
    let name = config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("restichub-agent");
    config_dir.with_file_name(format!("{name}-restore-roots.txt"))
}

/// Why the agent refuses a restore target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetDenied {
    /// The path is relative, holds `.` or `..`, or uses a form that the check does not model.
    NotAbsolute,
    /// The path cannot be resolved, for example a link that points nowhere.
    Unresolvable,
    /// The path is, holds or lies inside a file of the agent itself.
    ProtectedPath,
    /// Not below any configured root.
    OutsideRoots,
}

impl fmt::Display for TargetDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotAbsolute => "the path is not a plain absolute path",
            Self::Unresolvable => "the path cannot be resolved",
            Self::ProtectedPath => "the path overlaps a file of the agent",
            Self::OutsideRoots => "the path is outside the configured roots",
        })
    }
}

/// The restore roots and the paths of the agent that no restore may touch.
#[derive(Debug, Clone)]
pub struct RestoreAccess {
    roots: Vec<PathBuf>,
    protected: Vec<PathBuf>,
}

impl RestoreAccess {
    /// Builds the access from the configured roots and the protected paths.
    #[must_use]
    pub fn new(roots: Vec<PathBuf>, protected: Vec<PathBuf>) -> Self {
        Self { roots, protected }
    }

    /// The roots of the host (the default and the file) and the protected paths of this agent.
    ///
    /// `config_dir` is the identity directory, `cache_dir` the restic cache, and `program` the
    /// downloaded restic binary. The directory of `program` joins the protected paths (ADR-0038).
    #[must_use]
    pub fn for_host(config_dir: &Path, cache_dir: &Path, program: &Path) -> Self {
        let repository_roots = listed_roots(&crate::repository_roots::roots_file_for(config_dir));
        Self::new(
            load_roots(&roots_file_for(config_dir)),
            restore_protected(
                protected_paths(config_dir, cache_dir, program),
                repository_roots,
            ),
        )
    }

    /// Text form for `AgentHello`. A root that is not UTF-8 is left out.
    #[must_use]
    pub fn root_strings(&self) -> Vec<String> {
        self.roots
            .iter()
            .filter_map(|root| root.to_str().map(str::to_owned))
            .collect()
    }

    #[must_use]
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Resolves links first, so a link cannot lead out of a root or into a file of the agent.
    ///
    /// # Errors
    ///
    /// Returns the reason when the agent must refuse the target.
    pub fn check(&self, target: &str) -> Result<(), TargetDenied> {
        check_path(&self.roots, &self.protected, target)
    }
}

/// The files of the agent that no restore and no local repository may touch (T31).
///
/// `program` is the downloaded restic binary; its directory is protected too (ADR-0038).
pub(crate) fn protected_paths(config_dir: &Path, cache_dir: &Path, program: &Path) -> Vec<PathBuf> {
    let mut protected = vec![config_dir.to_path_buf(), cache_dir.to_path_buf()];
    protected.push(roots_file_for(config_dir));
    protected.push(crate::repository_roots::roots_file_for(config_dir));
    protected.push(crate::paths::program_dir());
    protected.push(crate::paths::data_dir());
    protected.push(crate::scripts::mode_file_for(config_dir));
    if let Some(dir) = program.parent() {
        protected.push(dir.to_path_buf());
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        protected.push(dir);
    }
    protected
}

/// A restore never writes into a repository root, even below a restore root `/` (ADR-0045).
fn restore_protected(mut protected: Vec<PathBuf>, repository_roots: Vec<PathBuf>) -> Vec<PathBuf> {
    protected.extend(repository_roots);
    protected
}

/// Accepts a path below one of `roots` that does not overlap a protected path. Links are resolved
/// first, so a link cannot lead out of a root or into a file of the agent.
pub(crate) fn check_path(
    roots: &[PathBuf],
    protected: &[PathBuf],
    target: &str,
) -> Result<(), TargetDenied> {
    let flavor = PathFlavor::host();
    if components(flavor, target).is_none() {
        return Err(TargetDenied::NotAbsolute);
    }
    let resolved = resolve(Path::new(target)).map_err(|_| TargetDenied::Unresolvable)?;
    let resolved = resolved.to_str().ok_or(TargetDenied::NotAbsolute)?;
    for protected in protected {
        let Some(path) = protected.to_str() else {
            continue;
        };
        let real = resolve(protected).ok();
        let real = real.as_deref().and_then(Path::to_str);
        if overlaps(flavor, target, path)
            || overlaps(flavor, resolved, path)
            || real.is_some_and(|real| overlaps(flavor, resolved, real))
        {
            return Err(TargetDenied::ProtectedPath);
        }
    }
    let inside = roots.iter().any(|root| {
        let Some(root) = resolve(root).ok() else {
            return false;
        };
        root.to_str()
            .is_some_and(|root| is_within(flavor, resolved, root))
    });
    if inside {
        Ok(())
    } else {
        Err(TargetDenied::OutsideRoots)
    }
}

/// Resolves the links of the deepest existing ancestor and appends the missing names.
///
/// A link that points nowhere is an error: the restore would create the file behind it.
pub(crate) fn resolve(path: &Path) -> io::Result<PathBuf> {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        match current.canonicalize() {
            Ok(base) => {
                return Ok(missing.iter().rev().fold(base, |mut path, name| {
                    path.push(name);
                    path
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(&current).is_ok() {
                    return Err(error);
                }
                let (Some(name), Some(parent)) = (current.file_name(), current.parent()) else {
                    return Err(error);
                };
                missing.push(name.to_owned());
                current = parent.to_path_buf();
            }
            Err(error) => return Err(error),
        }
    }
}

/// The default root and the roots of the file. A file that another account can change is ignored,
/// so a local user cannot widen the roots.
fn load_roots(file: &Path) -> Vec<PathBuf> {
    let mut roots = vec![default_root()];
    roots.extend(
        load_roots_file(file)
            .into_iter()
            .filter(|root| usable_restore_root(file, root)),
    );
    roots
}

fn usable_restore_root(file: &Path, root: &Path) -> bool {
    let home = cfg!(target_os = "linux") && crate::repository_roots::is_home_dir(root);
    if home {
        tracing::warn!(
            file = %file.display(),
            root = %root.display(),
            "the restore root is ignored: the service keeps /home and /root read-only; use a directory below it"
        );
    }
    !home
}

/// The safe roots of a roots file. An absent file holds no root.
pub(crate) fn load_roots_file(file: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    match std::fs::symlink_metadata(file) {
        Ok(_) => match read_trusted(file) {
            Ok(text) => {
                let (extra, skipped) = parse_roots(&text);
                if !skipped.is_empty() {
                    let lines: Vec<usize> = skipped.iter().map(|(line, _)| *line).collect();
                    tracing::warn!(
                        file = %file.display(),
                        lines = ?lines,
                        "the roots file holds lines that are not plain absolute paths"
                    );
                }
                for (_, root) in extra {
                    match check_root(&root) {
                        Ok(()) => roots.push(root),
                        Err(error) => tracing::warn!(
                            file = %file.display(),
                            root = %root.display(),
                            error = %format!("{error:#}"),
                            "the root is ignored"
                        ),
                    }
                }
            }
            Err(error) => tracing::warn!(
                file = %file.display(),
                error = %format!("{error:#}"),
                "the roots file is ignored"
            ),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(file = %file.display(), %error, "cannot read the roots file"),
    }
    roots
}

/// Every root line of a roots file, without the trust checks of `load_roots_file`.
///
/// Only for a refusal: a repository stays in a root that the agent stopped trusting, so a restore
/// must keep away from it (ADR-0045).
fn listed_roots(file: &Path) -> Vec<PathBuf> {
    // A FIFO or a device in place of the file would block or flood the read.
    if !std::fs::metadata(file).is_ok_and(|metadata| metadata.is_file()) {
        return Vec::new();
    }
    match std::fs::read_to_string(file) {
        Ok(text) => parse_roots(&text)
            .0
            .into_iter()
            .map(|(_, root)| root)
            .collect(),
        Err(error) => {
            tracing::warn!(file = %file.display(), %error, "cannot read the roots file");
            Vec::new()
        }
    }
}

/// The roots of the roots file that `load_roots` ignores, with the reason. `install-service`
/// refuses to continue while this list is not empty, so the administrator learns about the root.
#[must_use]
pub fn rejected_roots(config_dir: &Path) -> Vec<(PathBuf, String)> {
    rejected_roots_in(&roots_file_for(config_dir))
}

pub(crate) fn rejected_roots_in(file: &Path) -> Vec<(PathBuf, String)> {
    let Ok(text) = read_trusted(file) else {
        return Vec::new();
    };
    parse_roots(&text)
        .0
        .into_iter()
        .filter_map(|(_, root)| {
            let error = check_root(&root).err()?;
            Some((root, format!("{error:#}")))
        })
        .collect()
}

/// Refuses a line of a roots file that the agent skips at load, so `install-service` names it
/// instead of a silent warning.
///
/// # Errors
///
/// Returns an error with the line number for the first such line.
pub fn refuse_skipped_lines(file: &Path) -> Result<()> {
    match read_trusted(file) {
        Ok(text) => skipped_line_error(file, &text),
        Err(_) => Ok(()),
    }
}

fn skipped_line_error(file: &Path, text: &str) -> Result<()> {
    if let Some((line, text)) = parse_roots(text).1.into_iter().next() {
        anyhow::bail!(
            "line {line} of {} is not a plain absolute path, or holds a quote, a backslash, white space or %: {text:?}; fix or remove the line and run install-service again",
            file.display()
        );
    }
    Ok(())
}

/// Refuses the restore root `/home` or `/root`: `ProtectHome=read-only` keeps it read-only even in
/// `ReadWritePaths`, so a restore there would fail.
///
/// # Errors
///
/// Returns an error with the line number for the first such root.
pub fn refuse_home_roots(file: &Path) -> Result<()> {
    match read_trusted(file) {
        Ok(text) => home_root_error(file, &text),
        Err(_) => Ok(()),
    }
}

fn home_root_error(file: &Path, text: &str) -> Result<()> {
    if let Some((line, root)) = parse_roots(text)
        .0
        .into_iter()
        .find(|(_, root)| crate::repository_roots::is_home_dir(root))
    {
        anyhow::bail!(
            "line {line} of {}: the restore root {} stays read-only, because the service keeps /home and /root read-only; use a directory below it, owned by root, and run install-service again",
            file.display(),
            root.display()
        );
    }
    Ok(())
}

/// Refuses a root that another account can change, or whose existing ancestor another account can
/// change. Such an account could swap a directory for a link and lead a restore out of the root
/// (T31).
fn check_root(root: &Path) -> Result<()> {
    let resolved = resolve(root).with_context(|| format!("cannot resolve {}", root.display()))?;
    let mut current = Some(resolved.as_path());
    while let Some(path) = current {
        if std::fs::symlink_metadata(path).is_ok() {
            check_root_part(path, path == resolved)?;
        }
        current = path.parent();
    }
    Ok(())
}

#[cfg(unix)]
fn check_root_part(path: &Path, _is_root: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        anyhow::bail!(
            "{} must belong to root and must not be writable by a group or other accounts",
            path.display()
        );
    }
    if crate::acl::has_extended_acl(path)? {
        anyhow::bail!("{} holds an extended access control list", path.display());
    }
    Ok(())
}

#[cfg(windows)]
fn check_root_part(path: &Path, is_root: bool) -> Result<()> {
    if is_root {
        crate::winsec::check_admin_only(path)
    } else {
        crate::winsec::check_parent(path)
    }
}

#[cfg(not(any(unix, windows)))]
fn check_root_part(_path: &Path, _is_root: bool) -> Result<()> {
    Ok(())
}

/// Reads the roots file after the same ownership checks as the install location (T20).
fn read_trusted(file: &Path) -> Result<String> {
    check_trusted(file)?;
    std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))
}

#[cfg(unix)]
fn check_trusted(file: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = std::fs::symlink_metadata(file)
        .with_context(|| format!("cannot read {}", file.display()))?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        anyhow::bail!(
            "{} must be a regular file that only root can change",
            file.display()
        );
    }
    if crate::acl::has_extended_acl(file)? {
        anyhow::bail!("{} holds an extended access control list", file.display());
    }
    Ok(())
}

#[cfg(windows)]
fn check_trusted(file: &Path) -> Result<()> {
    crate::winsec::check_admin_only(file)
}

#[cfg(not(any(unix, windows)))]
fn check_trusted(_file: &Path) -> Result<()> {
    Ok(())
}

/// A line of a roots file with its number from 1.
type Numbered<T> = (usize, T);

/// The roots of a file text and the lines that are not plain absolute paths. A line that starts
/// with `#` is a comment.
fn parse_roots(text: &str) -> (Vec<Numbered<PathBuf>>, Vec<Numbered<String>>) {
    let mut roots = Vec::new();
    let mut skipped = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if components(PathFlavor::host(), line).is_some() && !has_unit_syntax(line) {
            roots.push((index + 1, PathBuf::from(line)));
        } else {
            skipped.push((index + 1, line.to_owned()));
        }
    }
    (roots, skipped)
}

/// Each root goes into `ReadWritePaths` of the systemd unit, where quotes, backslashes and white
/// space split or change the path and `%` starts a specifier. So `/srv" "/etc` would open `/etc`.
fn has_unit_syntax(line: &str) -> bool {
    cfg!(unix)
        && line
            .chars()
            .any(|c| matches!(c, '"' | '\'' | '\\' | '%') || c.is_whitespace() || c.is_control())
}

/// Creates the default root with the access of the service account only (FR-RS-03).
///
/// # Errors
///
/// Returns an error when the directory cannot be created or protected.
pub fn ensure_default_root() -> Result<()> {
    crate::paths::ensure_config_dir(&default_root())
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, windows))]
mod windows_tests;
