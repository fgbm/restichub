//! The profile preview: the steps and the command line with masked secrets
//! (FR-PF-04, AC-PF-01).

use restichub_proto::agent::v1::{
    HookPhase, Job, RepositoryAccess, ScriptInterpreter, ScriptStep, Step, step,
};
use restichub_proto::signing::Rejection;

use crate::arg::{self, ArgError};
use crate::compile::{Plan, PlannedStep};
use crate::job::ValidatedJob;

/// Why a plan cannot be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreviewError {
    #[error("the plan is not a valid job: {0:?}")]
    Rejected(Rejection),
    #[error(transparent)]
    Arg(#[from] ArgError),
}

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
/// Returns [`PreviewError`] when the plan is not a valid job or a step cannot
/// become a restic command.
pub fn preview(
    plan: &Plan,
    repository: &RepositoryAccess,
) -> Result<Vec<PreviewStep>, PreviewError> {
    let job = Job {
        job_id: "preview".to_owned(),
        tenant_id: "preview".to_owned(),
        agent_id: "preview".to_owned(),
        repositories: vec![repository.clone()],
        steps: plan
            .steps
            .iter()
            .map(|planned| planned.step.clone())
            .collect(),
        ..Job::default()
    };
    let validated = ValidatedJob::new(job).map_err(PreviewError::Rejected)?;

    plan.steps
        .iter()
        .map(|planned| preview_step(planned, &validated))
        .collect()
}

fn preview_step(
    planned: &PlannedStep,
    validated: &ValidatedJob,
) -> Result<PreviewStep, PreviewError> {
    if let Some(step::Kind::Script(script)) = &planned.step.kind {
        return Ok(PreviewStep {
            step: planned.step.clone(),
            maintenance: planned.maintenance,
            command: script_command(script),
            environment: Vec::new(),
        });
    }

    let mut line = arg::build_command(validated, &planned.step)?;
    arg::mask_secrets(&mut line);
    Ok(PreviewStep {
        step: planned.step.clone(),
        maintenance: planned.maintenance,
        command: render(&line),
        environment: line.env().to_vec(),
    })
}

/// The command line as the preview shows it. The agent writes the same text into the step log, so
/// the two agree (AC-PF-01).
#[must_use]
pub fn masked_command(line: &arg::CommandLine) -> String {
    let mut masked = line.clone();
    arg::mask_secrets(&mut masked);
    render(&masked)
}

/// The line of a script in the preview: its name, interpreter and phase, never the body or a value.
fn script_command(script: &ScriptStep) -> String {
    let interpreter = ScriptInterpreter::try_from(script.interpreter)
        .ok()
        .and_then(crate::script::ScriptInterpreter::from_proto);
    let names: Vec<&str> = script
        .env
        .iter()
        .map(|variable| variable.name.as_str())
        .collect();
    format!(
        "script={} interpreter={} phase={} env=[{}]",
        quote(&script.name),
        interpreter.map_or("unspecified", |value| match value {
            crate::script::ScriptInterpreter::Powershell => "powershell",
            crate::script::ScriptInterpreter::Cmd => "cmd",
            crate::script::ScriptInterpreter::Bash => "bash",
            crate::script::ScriptInterpreter::Sh => "sh",
        }),
        phase_name(script.phase),
        names.join(",")
    )
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
fn render(line: &arg::CommandLine) -> String {
    let mut parts = Vec::with_capacity(line.args().len() + 1);
    parts.push(line.program().to_owned());
    for arg in line.args() {
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
