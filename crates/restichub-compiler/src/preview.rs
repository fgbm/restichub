//! The profile preview: the steps and the command line with masked secrets
//! (FR-PF-04, AC-PF-01).

use std::collections::BTreeMap;

use restichub_proto::agent::v1::{HookPhase, HookStep, Job, RepositoryAccess, Step, step};
use restichub_proto::signing::Rejection;

use crate::arg::{self, ArgError, MASK};
use crate::compile::{Plan, PlannedStep};
use crate::job::ValidatedJob;
use crate::target::AllowListedHook;

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
/// The allow-list of the target agents supplies the secret flag of a hook
/// parameter, so the preview masks a secret hook parameter.
///
/// # Errors
///
/// Returns [`PreviewError`] when the plan is not a valid job or a step cannot
/// become a restic command.
pub fn preview(
    plan: &Plan,
    repository: &RepositoryAccess,
    allow_list: &BTreeMap<String, AllowListedHook>,
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
        .map(|planned| preview_step(planned, &validated, allow_list))
        .collect()
}

fn preview_step(
    planned: &PlannedStep,
    validated: &ValidatedJob,
    allow_list: &BTreeMap<String, AllowListedHook>,
) -> Result<PreviewStep, PreviewError> {
    if let Some(step::Kind::Hook(hook)) = &planned.step.kind {
        return Ok(PreviewStep {
            step: planned.step.clone(),
            maintenance: planned.maintenance,
            command: hook_command(hook, allow_list),
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

fn hook_command(hook: &HookStep, allow_list: &BTreeMap<String, AllowListedHook>) -> String {
    let schema = allow_list.get(&hook.hook_id);
    let mut parts = vec![
        format!("hook={}", hook.hook_id),
        format!("phase={}", phase_name(hook.phase)),
    ];
    for parameter in &hook.parameters {
        let secret = schema
            .and_then(|hook| hook.parameters.get(&parameter.name))
            .is_some_and(|spec| spec.secret);
        let value = if secret {
            MASK.to_owned()
        } else {
            quote(&parameter.value)
        };
        parts.push(format!("{}={}", parameter.name, value));
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
