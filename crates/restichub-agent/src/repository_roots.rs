//! Where a local repository may lie on a Linux host (ADR-0045).
//!
//! The systemd unit mounts the file system read-only, so restic writes a local repository only
//! below a path that `install-service` lists in `ReadWritePaths`. Only root configures these roots,
//! in a protected file like the restore roots (ADR-0035). A job and the control plane cannot change
//! them. Windows has no such sandbox, so the agent checks nothing there.

use std::path::{Path, PathBuf};

use restichub_proto::agent::v1::Job;
use restichub_proto::restore_path::{PathFlavor, components, is_within, overlaps};
use restichub_proto::validate::is_absolute_path;

use crate::restore_roots::{TargetDenied, check_path};

/// The file with the repository roots, one absolute path per line, next to the configuration
/// directory like the restore roots file.
#[must_use]
pub fn roots_file() -> PathBuf {
    roots_file_for(&crate::paths::config_dir())
}

pub(crate) fn roots_file_for(config_dir: &Path) -> PathBuf {
    let name = config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("restichub-agent");
    config_dir.with_file_name(format!("{name}-repository-roots.txt"))
}

/// The roots where a local repository may lie, or no check at all.
#[derive(Debug, Clone)]
pub struct RepositoryAccess {
    /// `None` on a host without the read-only sandbox.
    roots: Option<(Vec<PathBuf>, Vec<PathBuf>)>,
}

impl RepositoryAccess {
    /// Accepts a local repository below one of `roots` that does not overlap a `protected` path.
    #[must_use]
    pub fn new(roots: Vec<PathBuf>, protected: Vec<PathBuf>) -> Self {
        Self {
            roots: Some((roots, protected)),
        }
    }

    /// Accepts every local repository path.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self { roots: None }
    }

    /// The roots of the host file on Linux, where the service sandbox makes every other path
    /// read-only. Unrestricted on other systems.
    #[must_use]
    pub fn for_host(config_dir: &Path, cache_dir: &Path, program: &Path) -> Self {
        if cfg!(target_os = "linux") {
            let protected = crate::restore_roots::protected_paths(config_dir, cache_dir, program);
            Self::new(load_roots(config_dir, &protected), protected)
        } else {
            Self::unrestricted()
        }
    }

    #[must_use]
    pub fn roots(&self) -> &[PathBuf] {
        self.roots.as_ref().map_or(&[], |(roots, _)| roots)
    }

    /// Accepts the location of a local repository below a root.
    ///
    /// # Errors
    ///
    /// Returns the reason when the agent must refuse the path.
    pub fn check(&self, location: &str) -> Result<(), TargetDenied> {
        match &self.roots {
            Some((roots, protected)) => check_path(roots, protected, location),
            None => Ok(()),
        }
    }

    /// The first local repository of the job that the agent must refuse, with the reason.
    #[must_use]
    pub fn denied<'a>(&self, job: &'a Job) -> Option<(&'a str, TargetDenied)> {
        job.repositories
            .iter()
            .map(|repository| repository.location.as_str())
            .filter(|location| is_local(location))
            .find_map(|location| self.check(location).err().map(|reason| (location, reason)))
    }
}

/// A location without a backend prefix is a path. The validator allows only `s3:`, `rest:` and an
/// absolute path, and a Windows drive letter also has a colon.
pub(crate) fn is_local(location: &str) -> bool {
    is_absolute_path(location)
}

/// Directories that a repository root must not hold or lie in. Each root goes into
/// `ReadWritePaths`, so such a root would open the system files to a compromised restic.
const SYSTEM_DIRS: &[&str] = &[
    "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/boot", "/proc",
    "/sys", "/dev", "/run",
];

/// The directory that holds the state directories of the agent and the default restore root.
const STATE_PARENT: &str = "/var/lib";

/// The safe roots of the repository roots file. A root that is a system directory or overlaps a
/// file of the agent is left out with a warning.
pub(crate) fn load_roots(config_dir: &Path, protected: &[PathBuf]) -> Vec<PathBuf> {
    let file = roots_file_for(config_dir);
    crate::restore_roots::load_roots_file(&file)
        .into_iter()
        .filter(|root| match refused_root(root, protected) {
            Some(reason) => {
                tracing::warn!(
                    file = %file.display(),
                    line = %root.display(),
                    reason,
                    "the repository root is ignored"
                );
                false
            }
            None => true,
        })
        .collect()
}

/// Why a repository root is not allowed, or `None` for an allowed root.
pub(crate) fn refused_root(root: &Path, protected: &[PathBuf]) -> Option<&'static str> {
    let mut forms = vec![root.to_path_buf()];
    forms.extend(root.canonicalize().ok());
    let flavor = PathFlavor::host();
    for form in &forms {
        let Some(path) = form.to_str() else {
            return Some("the path is not UTF-8");
        };
        let Some(parts) = components(flavor, path) else {
            return Some("the path is not a plain absolute path");
        };
        if parts.is_empty() {
            return Some("the root is the root of the file system");
        }
        let system = SYSTEM_DIRS.iter().any(|dir| overlaps(flavor, path, dir));
        let state = is_within(flavor, STATE_PARENT, path)
            || (is_within(flavor, path, STATE_PARENT)
                && parts
                    .get(2)
                    .is_some_and(|name| name.starts_with("restichub")));
        if system || state {
            return Some("the root is a system directory, or holds or lies in one");
        }
        let agent = protected.iter().any(|protected| {
            protected
                .to_str()
                .is_some_and(|protected| overlaps(flavor, path, protected))
        });
        if agent {
            return Some("the root overlaps a file of the agent");
        }
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::expect_used)]

    use restichub_proto::agent::v1::RepositoryAccess as Access;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-repo-roots-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        dir.canonicalize().expect("cannot resolve the directory")
    }

    fn job(locations: &[&str]) -> Job {
        Job {
            repositories: locations
                .iter()
                .map(|location| Access {
                    location: (*location).to_owned(),
                    ..Access::default()
                })
                .collect(),
            ..Job::default()
        }
    }

    #[test]
    fn adr_0045_the_roots_file_is_a_sibling_of_the_config_dir() {
        assert_eq!(
            roots_file_for(Path::new("/etc/restichub-agent")),
            PathBuf::from("/etc/restichub-agent-repository-roots.txt")
        );
    }

    #[test]
    fn adr_0045_a_local_repository_outside_the_roots_is_refused() {
        let base = temp_dir("outside");
        let root = base.join("repos");
        std::fs::create_dir_all(&root).expect("cannot create the root");
        let access = RepositoryAccess::new(vec![root.clone()], vec![base.join("config")]);
        let inside = root.join("local-test");
        let outside = base.join("spool").join("local-test");
        let inside = inside.to_str().expect("utf-8");
        let outside = outside.to_str().expect("utf-8");
        assert_eq!(access.check(inside), Ok(()));
        assert_eq!(access.check(outside), Err(TargetDenied::OutsideRoots));
        assert_eq!(
            access.denied(&job(&["s3:s3.example.com/bucket", inside, outside])),
            Some((outside, TargetDenied::OutsideRoots))
        );
        assert_eq!(access.denied(&job(&[inside])), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn adr_0045_a_remote_repository_needs_no_root() {
        let access = RepositoryAccess::new(Vec::new(), Vec::new());
        assert_eq!(
            access.denied(&job(&[
                "s3:s3.example.com/bucket",
                "rest:https://rest.example.com/"
            ])),
            None
        );
        assert!(access.denied(&job(&["/srv/repo"])).is_some());
    }

    #[test]
    fn adr_0045_a_repository_on_the_files_of_the_agent_is_refused() {
        let base = temp_dir("protected");
        let config = base.join("config");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        let access = RepositoryAccess::new(vec![base.clone()], vec![config.clone()]);
        let target = config.join("repo");
        assert_eq!(
            access.check(target.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn adr_0045_an_unrestricted_access_accepts_every_path() {
        assert_eq!(RepositoryAccess::unrestricted().check("/srv/repo"), Ok(()));
        assert!(RepositoryAccess::unrestricted().roots().is_empty());
    }

    #[test]
    fn adr_0045_a_roots_file_that_another_account_can_change_is_ignored() {
        use std::os::unix::fs::PermissionsExt as _;
        let base = temp_dir("untrusted");
        let file = base.join("roots.txt");
        std::fs::write(&file, "/srv/repos\n").expect("cannot write the file");
        // A world-writable file fails the mode check under root and the owner check otherwise.
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666))
            .expect("cannot set the mode");
        assert!(crate::restore_roots::load_roots_file(&file).is_empty());
        assert!(crate::restore_roots::load_roots_file(&base.join("absent.txt")).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn adr_0045_the_filesystem_root_and_system_directories_are_refused_as_roots() {
        let base = temp_dir("system-roots");
        let config = base.join("config");
        let protected = vec![config.clone(), PathBuf::from("/opt/restichub-agent")];
        for root in [
            "/",
            "//",
            "/etc",
            "/etc/backups",
            "/usr/local/repos",
            "/boot",
            "/proc",
            "/sys",
            "/dev",
            "/run/repos",
            "/lib64",
            "/var",
            "/var/lib",
            "/var/lib/restichub-restore",
            "/var/lib/restichub-agent/repos",
            "/opt",
            "/opt/restichub-agent",
        ] {
            assert!(
                refused_root(Path::new(root), &protected).is_some(),
                "{root} must be refused"
            );
        }
        assert!(refused_root(&config.join("repos"), &protected).is_some());
        for root in [
            "/srv/repos",
            "/var/spool/restichub",
            "/var/lib/backups",
            "/home",
        ] {
            assert_eq!(refused_root(Path::new(root), &protected), None, "{root}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn adr_0045_the_repository_roots_file_is_a_protected_path() {
        let base = temp_dir("roots-file");
        let config = base.join("config");
        std::fs::create_dir_all(&config).expect("cannot create the config");
        let restore = crate::restore_roots::RestoreAccess::new(
            vec![base.clone()],
            crate::restore_roots::protected_paths(
                &config,
                &base.join("cache"),
                &base.join("restic"),
            ),
        );
        let file = roots_file_for(&config);
        assert_eq!(
            restore.check(file.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath)
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
