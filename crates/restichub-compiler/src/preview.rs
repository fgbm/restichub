//! The profile preview: the steps and the command line with masked secrets
//! (FR-PF-04, AC-PF-01).

use restichub_proto::agent::v1::{HookPhase, HookStep, RepositoryAccess, Step, step};

use crate::arg::{self, ArgError, CommandLine};
use crate::compile::{Plan, PlannedStep};

/// One previewed step: the typed step, its command line and its environment.
/// Secrets in the environment are masked.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewStep {
    pub step: Step,
    pub maintenance: bool,
    /// Rendered argv for display, with no secret.
    pub command: String,
    /// Process variables, with secrets masked.
    pub environment: Vec<(String, String)>,
}

/// Renders a plan as step command lines for one repository (AC-PF-01).
///
/// # Errors
///
/// Returns an [`ArgError`] when a step cannot become a restic command.
pub fn preview(plan: &Plan, repository: &RepositoryAccess) -> Result<Vec<PreviewStep>, ArgError> {
    plan.steps
        .iter()
        .map(|planned| preview_step(planned, repository))
        .collect()
}

fn preview_step(
    planned: &PlannedStep,
    repository: &RepositoryAccess,
) -> Result<PreviewStep, ArgError> {
    if let Some(step::Kind::Hook(hook)) = &planned.step.kind {
        return Ok(PreviewStep {
            step: planned.step.clone(),
            maintenance: planned.maintenance,
            command: hook_command(hook),
            environment: Vec::new(),
        });
    }

    let mut line = arg::build_command(&planned.step, repository)?;
    arg::mask_secrets(&mut line);
    Ok(PreviewStep {
        step: planned.step.clone(),
        maintenance: planned.maintenance,
        command: render(&line),
        environment: line.env,
    })
}

fn hook_command(hook: &HookStep) -> String {
    let mut parts = vec![
        format!("hook={}", hook.hook_id),
        format!("phase={}", phase_name(hook.phase)),
    ];
    for parameter in &hook.parameters {
        parts.push(format!("{}={}", parameter.name, parameter.value));
    }
    parts.join(" ")
}

fn phase_name(phase: i32) -> &'static str {
    match HookPhase::try_from(phase) {
        Ok(HookPhase::Before) => "before",
        Ok(HookPhase::After) => "after",
        Ok(HookPhase::OnSuccess) => "on_success",
        Ok(HookPhase::OnError) => "on_error",
        Ok(HookPhase::Finally) => "finally",
        _ => "unspecified",
    }
}

/// Formats argv for a person. The quoting is for display only; the agent
/// starts the process without a shell (FR-PR-10).
fn render(line: &CommandLine) -> String {
    let mut parts = Vec::with_capacity(line.args.len() + 1);
    parts.push(line.program.clone());
    for arg in &line.args {
        parts.push(quote(arg));
    }
    parts.join(" ")
}

fn quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_owned();
    }
    let simple = arg.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'/' | b'.' | b'_' | b'-' | b'=' | b':' | b',' | b'%' | b'+' | b'@'
            )
    });
    if simple {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}
