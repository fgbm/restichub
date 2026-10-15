//! Fixtures for the hook tests: a scripted hook program, an allow-list file and the step builders.

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use restichub_agent::hooks::{HookStore, Trust};
use restichub_agent::jobs::Jobs;
use restichub_proto::agent::v1::{
    HookParameter, HookPhase, HookStep, Job, RepositoryAccess, Step, hook_parameter::Typed, step,
};

use super::steps::{access, job, make_step};
use super::{program_jobs, temp_config_dir};

pub const PASSWORD: &str = "CANARY-hook-repository-password";

/// The path of the `fake-hook` example next to the test binary.
#[must_use]
pub fn fake_hook() -> PathBuf {
    let exe = std::env::current_exe().expect("cannot find the test binary");
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("the test binary holds a target directory");
    let name = if cfg!(windows) {
        "fake-hook.exe"
    } else {
        "fake-hook"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.exists(),
        "the fake-hook example must be built: {}",
        path.display()
    );
    path
}

/// An agent directory, a work directory with the control files of the hooks and an allow-list.
pub struct HookFixture {
    pub dir: PathBuf,
    pub work: PathBuf,
    pub repository: PathBuf,
    /// A copy of the fake program with the mode of a trusted executable: the build directory
    /// takes the umask of the developer.
    pub executable: PathBuf,
}

impl HookFixture {
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
        let executable = work.join(if cfg!(windows) {
            "fake-hook.exe"
        } else {
            "fake-hook"
        });
        std::fs::copy(fake_hook(), &executable).expect("cannot copy the fake hook");
        set_mode(&executable, 0o755);
        Self {
            dir,
            work,
            repository,
            executable,
        }
    }

    /// Where the fixture writes the allow-list.
    #[must_use]
    pub fn allow_list(&self) -> PathBuf {
        self.work.join("hooks.toml")
    }

    /// Writes the allow-list with the mode of a trusted file.
    pub fn write_allow_list(&self, text: &str) {
        let path = self.allow_list();
        std::fs::write(&path, text).expect("cannot write the allow-list");
        set_mode(&path, 0o644);
    }

    /// The text of one hook: the fake program with its control file as the first argument.
    #[must_use]
    pub fn hook(&self, id: &str, extra: &str) -> String {
        self.hook_with(&self.executable, id, extra)
    }

    #[must_use]
    pub fn hook_with(&self, executable: &Path, id: &str, extra: &str) -> String {
        format!(
            "[[hook]]\nid = '{id}'\nexecutable = '{}'\nargs = ['--control={}']\n{extra}\n",
            executable.display(),
            self.control(id).display(),
        )
    }

    /// The control file of a hook: the script that the fake program follows.
    #[must_use]
    pub fn control(&self, id: &str) -> PathBuf {
        self.work.join(format!("{id}.control"))
    }

    pub fn script(&self, id: &str, text: &str) {
        std::fs::write(self.control(id), text).expect("cannot write the control file");
    }

    /// What the fake program recorded for a hook: one block for each run.
    #[must_use]
    pub fn record(&self, id: &str) -> String {
        std::fs::read_to_string(format!("{}.record", self.control(id).display()))
            .unwrap_or_default()
    }

    #[must_use]
    pub fn runs(&self, id: &str) -> usize {
        self.record(id)
            .lines()
            .filter(|line| *line == "run")
            .count()
    }

    /// What the program saw in `argv`, without the fixed control argument.
    #[must_use]
    pub fn arguments(&self, id: &str) -> Vec<String> {
        self.record(id)
            .lines()
            .filter_map(|line| line.strip_prefix("arg:"))
            .map(str::to_owned)
            .collect()
    }

    /// Variables of the child process, as the program recorded them.
    #[must_use]
    pub fn environment(&self, id: &str) -> Vec<(String, String)> {
        self.record(id)
            .lines()
            .filter_map(|line| line.strip_prefix("env:"))
            .filter_map(|line| line.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    /// The jobs of the fixture with the allow-list under the trust of the current user.
    #[must_use]
    pub fn jobs(&self) -> Jobs {
        self.jobs_with(Trust::CurrentUser)
    }

    #[must_use]
    pub fn jobs_with(&self, trust: Trust) -> Jobs {
        program_jobs(&self.dir).with_hook_store(HookStore::new(self.allow_list(), trust))
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

impl Drop for HookFixture {
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

/// Builds a step that names one allow-list entry and its phase.
#[must_use]
pub fn hook_step(
    step_id: &str,
    hook_id: &str,
    phase: HookPhase,
    parameters: Vec<HookParameter>,
) -> Step {
    make_step(
        step_id,
        step::Kind::Hook(HookStep {
            hook_id: hook_id.to_owned(),
            phase: phase as i32,
            parameters,
        }),
    )
}

#[must_use]
pub fn string_param(name: &str, value: &str) -> HookParameter {
    HookParameter {
        name: name.to_owned(),
        typed: Some(Typed::StringValue(value.to_owned())),
        ..HookParameter::default()
    }
}

#[must_use]
pub fn integer_param(name: &str, value: i64) -> HookParameter {
    HookParameter {
        name: name.to_owned(),
        typed: Some(Typed::IntegerValue(value)),
        ..HookParameter::default()
    }
}

#[must_use]
pub fn bool_param(name: &str, value: bool) -> HookParameter {
    HookParameter {
        name: name.to_owned(),
        typed: Some(Typed::BooleanValue(value)),
        ..HookParameter::default()
    }
}

#[must_use]
pub fn secret_param(name: &str, value: &str) -> HookParameter {
    HookParameter {
        name: name.to_owned(),
        typed: Some(Typed::SecretValue(value.to_owned())),
        ..HookParameter::default()
    }
}
