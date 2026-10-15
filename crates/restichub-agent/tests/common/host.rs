//! Fixtures for the script tests: an agent directory, a work directory and the fake restic.

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use restichub_agent::jobs::Jobs;
use restichub_proto::agent::v1::{Job, RepositoryAccess, Step};

use super::steps::{access, job};
use super::{program_jobs, temp_config_dir};

pub const PASSWORD: &str = "CANARY-hook-repository-password";

/// An agent directory, a work directory and a repository path.
pub struct HostFixture {
    pub dir: PathBuf,
    pub work: PathBuf,
    pub repository: PathBuf,
}

impl HostFixture {
    #[must_use]
    pub fn new(name: &str) -> Self {
        let dir = temp_config_dir(&format!("{name}-agent"));
        std::fs::create_dir_all(&dir).expect("cannot create the configuration directory");
        let work = temp_config_dir(&format!("{name}-work"));
        std::fs::create_dir_all(&work).expect("cannot create the work directory");
        let repository = temp_config_dir(&format!("{name}-repo"));
        std::fs::create_dir_all(&repository).expect("cannot create the repository path");
        make_private(&dir);
        make_private(&work);
        Self {
            dir,
            work,
            repository,
        }
    }

    #[must_use]
    pub fn jobs(&self) -> Jobs {
        program_jobs(&self.dir)
    }

    #[must_use]
    pub fn repositories(&self) -> Vec<RepositoryAccess> {
        vec![access("repo-1", &self.repository, PASSWORD)]
    }

    #[must_use]
    pub fn job(&self, job_id: &str, steps: Vec<Step>) -> Job {
        job(job_id, self.repositories(), steps)
    }

    /// The arguments of the fake restic, one run on each line.
    #[must_use]
    pub fn restic_runs(&self) -> Vec<String> {
        super::recorded_arguments(&self.repository)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Makes the fake restic exit with the code for the command.
    pub fn restic_code(&self, command: &str, code: i32) {
        std::fs::write(
            format!("{}.restichub-code-{command}", self.repository.display()),
            code.to_string(),
        )
        .expect("cannot write the code");
    }
}

impl Drop for HostFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.work);
        let _ = std::fs::remove_dir_all(&self.repository);
    }
}

#[cfg(unix)]
pub fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .expect("cannot set the mode");
}

#[cfg(not(unix))]
pub fn set_mode(_path: &Path, _mode: u32) {}

fn make_private(path: &Path) {
    set_mode(path, 0o755);
}
