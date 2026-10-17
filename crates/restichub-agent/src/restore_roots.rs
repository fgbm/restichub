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
        let protected = protected_paths(config_dir, cache_dir, program);
        let repository_roots = crate::repository_roots::load_roots(config_dir, &protected);
        Self::new(
            load_roots(&roots_file_for(config_dir)),
            restore_protected(protected, repository_roots),
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

/// A restore never writes into a repository root, even below a restore root `/` (ADR-0044).
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
fn resolve(path: &Path) -> io::Result<PathBuf> {
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
    roots.extend(load_roots_file(file));
    roots
}

/// The safe roots of a roots file. An absent file holds no root.
pub(crate) fn load_roots_file(file: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    match std::fs::symlink_metadata(file) {
        Ok(_) => match read_trusted(file) {
            Ok(text) => {
                let (extra, skipped) = parse_roots(&text);
                if skipped > 0 {
                    tracing::warn!(
                        file = %file.display(),
                        skipped,
                        "the roots file holds lines that are not absolute paths"
                    );
                }
                for root in extra {
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
        .filter_map(|root| {
            let error = check_root(&root).err()?;
            Some((root, format!("{error:#}")))
        })
        .collect()
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

/// The roots of a file text and the number of lines that are not absolute paths. A line that starts
/// with `#` is a comment.
fn parse_roots(text: &str) -> (Vec<PathBuf>, usize) {
    let mut roots = Vec::new();
    let mut skipped = 0;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with('#') {
            continue;
        }
        if components(PathFlavor::host(), line).is_some() {
            roots.push(PathBuf::from(line));
        } else {
            skipped += 1;
        }
    }
    (roots, skipped)
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
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-roots-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        dir.canonicalize().expect("cannot resolve the directory")
    }

    fn access(root: &Path, config: &Path) -> RestoreAccess {
        RestoreAccess::new(
            vec![root.to_path_buf()],
            vec![config.to_path_buf(), PathBuf::from("/etc/restichub-agent")],
        )
    }

    #[test]
    fn ac_rs_05_the_default_root_is_below_var_lib() {
        assert_eq!(default_root(), PathBuf::from("/var/lib/restichub-restore"));
    }

    #[test]
    fn ac_rs_05_a_target_inside_a_root_is_accepted() {
        let base = temp_dir("inside");
        let root = base.join("restore");
        let config = base.join("config");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        let access = access(&root, &config);
        let target = root.join("job-1");
        assert_eq!(access.check(target.to_str().expect("utf-8")), Ok(()));
        let deep = root.join("a").join("b");
        assert_eq!(access.check(deep.to_str().expect("utf-8")), Ok(()));
        assert_eq!(access.check(root.to_str().expect("utf-8")), Ok(()));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_the_filesystem_root_and_system_paths_are_refused() {
        let base = temp_dir("system");
        let root = base.join("restore");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        let access = access(&root, &base.join("config"));
        assert!(access.check("/").is_err());
        assert_eq!(access.check("/etc"), Err(TargetDenied::ProtectedPath));
        assert_eq!(access.check("/etc/passwd"), Err(TargetDenied::OutsideRoots));
        assert_eq!(
            access.check("relative/path"),
            Err(TargetDenied::NotAbsolute)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_a_path_outside_the_roots_is_refused() {
        let base = temp_dir("outside");
        let root = base.join("restore");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        let access = access(&root, &base.join("config"));
        let other = base.join("other");
        assert_eq!(
            access.check(other.to_str().expect("utf-8")),
            Err(TargetDenied::OutsideRoots)
        );
        let sibling = base.join("restore2");
        assert_eq!(
            access.check(sibling.to_str().expect("utf-8")),
            Err(TargetDenied::OutsideRoots)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_the_config_dir_and_what_lies_inside_or_above_it_are_refused() {
        let base = temp_dir("config");
        let config = base.join("restore").join("config");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        // A root that holds the config dir shows that the protected rule wins over the root rule.
        let access = access(&base, &config);
        for target in [
            config.clone(),
            config.join("identity.json"),
            base.join("restore"),
            base.clone(),
        ] {
            assert_eq!(
                access.check(target.to_str().expect("utf-8")),
                Err(TargetDenied::ProtectedPath),
                "{}",
                target.display()
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn t20_the_restic_directory_is_a_protected_path() {
        let base = temp_dir("restic-dir");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).expect("cannot create the bin directory");
        let access = RestoreAccess::for_host(
            &base.join("config"),
            &base.join("cache"),
            &bin.join("restic"),
        );
        assert_eq!(
            access.check(bin.join("restic").to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn t20_the_program_and_data_directories_are_protected_paths() {
        let base = temp_dir("program-dir");
        let access = RestoreAccess::for_host(
            &base.join("config"),
            &base.join("cache"),
            &base.join("bin").join("restic"),
        );
        for dir in [crate::paths::program_dir(), crate::paths::data_dir()] {
            let target = dir.join("restore");
            assert_eq!(
                access.check(target.to_str().expect("utf-8")),
                Err(TargetDenied::ProtectedPath),
                "{}",
                target.display()
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_a_link_to_the_config_dir_is_refused() {
        let base = temp_dir("link");
        let root = base.join("restore");
        let config = base.join("config");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        std::os::unix::fs::symlink(&config, root.join("link")).expect("cannot create the link");
        let access = access(&root, &config);
        for target in [root.join("link"), root.join("link").join("new")] {
            assert_eq!(
                access.check(target.to_str().expect("utf-8")),
                Err(TargetDenied::ProtectedPath),
                "{}",
                target.display()
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_a_link_out_of_the_root_is_refused() {
        let base = temp_dir("escape");
        let root = base.join("restore");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        std::fs::create_dir_all(&outside).expect("cannot create the outside");
        std::os::unix::fs::symlink(&outside, root.join("link")).expect("cannot create the link");
        std::os::unix::fs::symlink(base.join("missing"), root.join("dangling"))
            .expect("cannot create the link");
        let access = access(&root, &base.join("config"));
        let escaped = root.join("link").join("x");
        assert_eq!(
            access.check(escaped.to_str().expect("utf-8")),
            Err(TargetDenied::OutsideRoots)
        );
        let dangling = root.join("dangling").join("x");
        assert_eq!(
            access.check(dangling.to_str().expect("utf-8")),
            Err(TargetDenied::Unresolvable)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_a_dot_dot_component_is_refused() {
        let base = temp_dir("dotdot");
        let root = base.join("restore");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        let access = access(&root, &base.join("config"));
        let target = format!("{}/../config", root.display());
        assert_eq!(access.check(&target), Err(TargetDenied::NotAbsolute));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_a_root_of_slash_allows_an_in_place_restore_but_not_the_agent_files() {
        let base = temp_dir("slash");
        let config = base.join("config");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        let access = RestoreAccess::new(vec![PathBuf::from("/")], vec![config.clone()]);
        let data = base.join("data");
        assert_eq!(access.check(data.to_str().expect("utf-8")), Ok(()));
        assert_eq!(
            access.check(config.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath)
        );
        assert_eq!(access.check("/"), Err(TargetDenied::ProtectedPath));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn adr_0044_a_restore_into_a_repository_root_is_refused() {
        let base = temp_dir("repository-root");
        let config = base.join("config");
        let repos = base.join("repos");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        std::fs::create_dir_all(repos.join("fs-01")).expect("cannot create the repository");
        let protected = restore_protected(vec![config], vec![repos.clone()]);
        for root in [PathBuf::from("/"), base.clone()] {
            let access = RestoreAccess::new(vec![root.clone()], protected.clone());
            for target in [
                repos.clone(),
                repos.join("fs-01").join("config"),
                base.clone(),
            ] {
                assert_eq!(
                    access.check(target.to_str().expect("utf-8")),
                    Err(TargetDenied::ProtectedPath),
                    "{} below {}",
                    target.display(),
                    root.display()
                );
            }
            let other = base.join("data");
            assert_eq!(access.check(other.to_str().expect("utf-8")), Ok(()));
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_the_roots_file_lists_absolute_paths() {
        let (roots, skipped) =
            parse_roots("# comment\n\n/srv/restore\n  /data  \nrelative\n../x\n");
        assert_eq!(
            roots,
            vec![PathBuf::from("/srv/restore"), PathBuf::from("/data")]
        );
        assert_eq!(skipped, 2);
    }

    #[test]
    fn ac_rs_05_a_roots_file_of_a_user_account_is_ignored() {
        let base = temp_dir("untrusted");
        let file = base.join("roots.txt");
        std::fs::write(&file, "/srv/extra\n").expect("cannot write the file");
        // The check compares the owner with root, so the test needs a non-root account.
        let owner = std::os::unix::fs::MetadataExt::uid(
            &std::fs::metadata(&file).expect("cannot read the file"),
        );
        if owner == 0 {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let roots = load_roots(&file);
        assert_eq!(roots, vec![default_root()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_a_root_writable_by_others_is_ignored() {
        use std::os::unix::fs::PermissionsExt as _;
        let base = temp_dir("writable-root");
        let root = base.join("open");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777))
            .expect("cannot set the mode");
        assert!(check_root(&root).is_err());
        // A directory below the open root fails through its ancestor.
        let below = root.join("below");
        std::fs::create_dir_all(&below).expect("cannot create the directory");
        std::fs::set_permissions(&below, std::fs::Permissions::from_mode(0o700))
            .expect("cannot set the mode");
        assert!(check_root(&below).is_err());
        assert!(check_root(&root.join("missing").join("x")).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_hk_02_a_restore_cannot_touch_the_trust_mode_file() {
        let base = temp_dir("trust-file");
        let config = base.join("config");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        let access = RestoreAccess::for_host(&config, &base.join("cache"), &base.join("restic"));
        let mode_file = crate::scripts::mode_file_for(&config);
        assert_eq!(
            access.check(mode_file.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ac_rs_05_the_roots_file_is_a_sibling_of_the_config_dir() {
        assert_eq!(
            roots_file_for(Path::new("/etc/restichub-agent")),
            PathBuf::from("/etc/restichub-agent-restore-roots.txt")
        );
    }

    #[test]
    fn ac_rs_05_the_roots_are_reported_as_text() {
        let access = RestoreAccess::new(vec![PathBuf::from("/srv/restore")], Vec::new());
        assert_eq!(access.root_strings(), vec!["/srv/restore".to_owned()]);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn ac_rs_05_the_default_root_is_outside_the_agent_tree() {
        let agent_root = crate::paths::config_dir()
            .parent()
            .map(Path::to_path_buf)
            .expect("the configuration directory has a parent");
        assert!(!default_root().starts_with(&agent_root));
        assert_eq!(
            default_root().file_name().and_then(|name| name.to_str()),
            Some("ResticHub-Restore")
        );
    }
}
