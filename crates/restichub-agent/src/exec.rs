//! Hardening of the restic child process (FR-AG-06, ADR-0021, ADR-0031).
//!
//! The agent starts the bundled restic with a cleared environment and only the variables of the step
//! `CommandLine` (FR-AG-06). A secret reaches the child process only through the environment, never
//! through an argument (threat T9). The hash of the binary is re-checked immediately before the
//! spawn, so a binary that changed after the agent start is refused (threat T20).

use std::path::Path;
use std::process::{Child, Command};

use anyhow::Result;
use restichub_compiler::arg::CommandLine;

use crate::restic::{Restic, ResticBinary};

/// Builds the restic invocation with a cleared environment and only the step values (FR-AG-06).
///
/// The child process inherits no variable of the agent, so an ambient secret never reaches restic.
#[must_use]
pub fn restic_command(program: &Path, line: &CommandLine) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    for (name, value) in line.env() {
        command.env(name, value);
    }
    command.args(line.args());
    command
}

/// Re-checks the pinned hash and starts the child process (FR-AG-11, threat T20).
///
/// The check runs immediately before the spawn, so a binary that an attacker replaced between the
/// agent start and the step is refused.
///
/// # Errors
///
/// Returns an error when the hash differs or the process cannot start.
pub fn spawn_restic(restic: &ResticBinary, line: &CommandLine) -> Result<Child> {
    restic.verify()?;
    Ok(restic_command(restic.path(), line).spawn()?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    use restichub_compiler::ValidatedJob;
    use restichub_compiler::arg::build_command;
    use restichub_proto::agent::v1::{Job, RepositoryAccess, Step, UnlockStep, step};

    use super::restic_command;

    fn command_line() -> restichub_compiler::CommandLine {
        let job = Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![RepositoryAccess {
                repository_id: "repo-1".to_owned(),
                location: "s3:s3.example.com/bucket".to_owned(),
                password: "topsecret".to_owned(),
                ..RepositoryAccess::default()
            }],
            steps: vec![Step {
                step_id: "unlock".to_owned(),
                kind: Some(step::Kind::Unlock(UnlockStep {
                    repository_id: "repo-1".to_owned(),
                })),
                ..Step::default()
            }],
            ..Job::default()
        };
        let validated = ValidatedJob::new(job).expect("the job must validate");
        let step = validated.job().steps[0].clone();
        build_command(&validated, &step).expect("the command must build")
    }

    /// FR-AG-06: the child process holds only the step environment and no argument carries a secret.
    #[test]
    fn fr_ag_06_the_child_environment_is_cleared() {
        let line = command_line();
        let program = Path::new("/opt/restichub-agent/restic");
        let command = restic_command(program, &line);

        assert_eq!(
            command.get_program(),
            OsStr::new("/opt/restichub-agent/restic")
        );
        let args: Vec<&OsStr> = command.get_args().collect();
        let expected_args: Vec<&OsStr> = line.args().iter().map(OsStr::new).collect();
        assert_eq!(args, expected_args);

        let mut env: Vec<(OsString, Option<OsString>)> = command
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(OsString::from)))
            .collect();
        env.sort();
        let mut expected_env: Vec<(OsString, Option<OsString>)> = line
            .env()
            .iter()
            .map(|(name, value)| (OsString::from(name), Some(OsString::from(value))))
            .collect();
        expected_env.sort();
        assert_eq!(env, expected_env);

        // No variable of the parent process is inherited.
        assert!(
            !command
                .get_envs()
                .any(|(name, _)| name == OsStr::new("PATH"))
        );
        assert!(
            !command
                .get_envs()
                .any(|(name, _)| name == OsStr::new("HOME"))
        );
    }
}
