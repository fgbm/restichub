//! The command line that opens the log of a step (AC-PF-01).

use restichub_compiler::{CommandLine, OperatingSystem, masked_command};

use crate::scrub::Scrubber;

/// The longest command line in the log. A profile can hold paths for megabytes, and a log chunk
/// over the limit of the control plane is dropped.
const COMMAND_LIMIT: usize = 4 * 1024;

const fn host_os() -> OperatingSystem {
    if cfg!(windows) {
        OperatingSystem::Windows
    } else if cfg!(target_os = "macos") {
        OperatingSystem::Macos
    } else {
        OperatingSystem::Linux
    }
}

/// The line `$ restic ...` with the argv that the process gets, scrubbed like the output of the
/// process and cut to [`COMMAND_LIMIT`] bytes.
pub(super) fn command_line_text(scrubber: &Scrubber, line: &CommandLine) -> String {
    let text = scrubber.scrub(&masked_command(line, host_os()));
    if text.len() <= COMMAND_LIMIT {
        return format!("$ {text}");
    }
    let mut end = COMMAND_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("$ {} ... ({} more bytes)", &text[..end], text.len() - end)
}

#[cfg(test)]
mod tests {
    use restichub_compiler::ValidatedJob;
    use restichub_compiler::arg::build_command;
    use restichub_proto::agent::v1::{BackupStep, Job, RepositoryAccess, Step, step};

    use super::{COMMAND_LIMIT, command_line_text};
    use crate::scrub::Scrubber;

    fn line(paths: Vec<String>, password: &str) -> restichub_compiler::CommandLine {
        let step = Step {
            step_id: "backup".to_owned(),
            kind: Some(step::Kind::Backup(BackupStep {
                repository_id: "repo-1".to_owned(),
                paths,
                ..BackupStep::default()
            })),
            ..Step::default()
        };
        let job = Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![RepositoryAccess {
                repository_id: "repo-1".to_owned(),
                location: "/srv/repo".to_owned(),
                password: password.to_owned(),
                ..RepositoryAccess::default()
            }],
            steps: vec![step.clone()],
            ..Job::default()
        };
        let validated = ValidatedJob::new(job).expect("the job must validate");
        build_command(&validated, &step)
            .expect("the command must build")
            .with_json()
    }

    #[test]
    fn ac_pf_01_the_logged_command_is_the_argv_with_json() {
        let line = line(vec!["/srv/data".to_owned()], "CANARY-password");
        let text = command_line_text(&Scrubber::for_command(&line), &line);
        assert!(text.starts_with("$ restic backup --json "), "{text}");
        assert!(text.contains("/srv/data"), "{text}");
    }

    #[test]
    fn ac_lg_04_the_logged_command_passes_the_scrubber() {
        let line = line(
            vec!["/srv/CANARY-password/data".to_owned()],
            "CANARY-password",
        );
        let text = command_line_text(&Scrubber::for_command(&line), &line);
        assert!(!text.contains("CANARY-password"), "{text}");
    }

    #[test]
    fn ac_pf_01_a_long_command_is_cut_with_a_marker() {
        let paths = (0..256)
            .map(|n| format!("/srv/{n}/{}", "d".repeat(2000)))
            .collect();
        let line = line(paths, "CANARY-password");
        let text = command_line_text(&Scrubber::for_command(&line), &line);
        assert!(text.len() < COMMAND_LIMIT + 64, "{}", text.len());
        assert!(text.starts_with("$ restic backup --json "), "{text}");
        assert!(text.contains("... ("), "{text}");
    }
}
