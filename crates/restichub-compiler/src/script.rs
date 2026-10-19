//! Scripts of a profile: the input types and the steps (FR-HK-09, ADR-0037).
//!
//! A script is code that the control plane sends. The compiler checks only what the target agent
//! reports (trust mode and interpreters) and turns the script into a typed step. The agent checks
//! it again before it runs the script.

use restichub_proto::agent::v1::{
    HookPhase as ProtoHookPhase, ScriptEnv, ScriptFailurePolicy as ProtoPolicy,
    ScriptInterpreter as ProtoInterpreter, ScriptStep, Step, script_env, step,
};
use serde::{Deserialize, Serialize};

use crate::error::{CompileError, FieldPath};
use crate::profile::HookPhase;
use crate::target::{OperatingSystem, ParameterValue, TargetFacts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptInterpreter {
    Powershell,
    Cmd,
    Bash,
    Sh,
}

impl ScriptInterpreter {
    /// Every interpreter, in the order of the user interface.
    pub const ALL: [Self; 4] = [Self::Powershell, Self::Cmd, Self::Bash, Self::Sh];

    /// The interpreters that exist on an operating system.
    #[must_use]
    pub fn for_os(os: OperatingSystem) -> &'static [Self] {
        match os {
            OperatingSystem::Windows => &[Self::Powershell, Self::Cmd],
            _ => &[Self::Bash, Self::Sh],
        }
    }

    #[must_use]
    pub fn to_proto(self) -> ProtoInterpreter {
        match self {
            Self::Powershell => ProtoInterpreter::Powershell,
            Self::Cmd => ProtoInterpreter::Cmd,
            Self::Bash => ProtoInterpreter::Bash,
            Self::Sh => ProtoInterpreter::Sh,
        }
    }

    /// The interpreter of a wire value, or `None` for the unspecified value.
    #[must_use]
    pub fn from_proto(value: ProtoInterpreter) -> Option<Self> {
        match value {
            ProtoInterpreter::Powershell => Some(Self::Powershell),
            ProtoInterpreter::Cmd => Some(Self::Cmd),
            ProtoInterpreter::Bash => Some(Self::Bash),
            ProtoInterpreter::Sh => Some(Self::Sh),
            ProtoInterpreter::Unspecified => None,
        }
    }
}

/// What a failed script does to the job (FR-HK-09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    AbortBackup,
    /// The job goes on. The job still ends as failed.
    Continue,
}

/// The value of one environment variable of a script (FR-HK-09, FR-PF-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptEnvValue {
    Plain(String),
    /// The identifier of a secret. The control plane replaces it when it sends the job.
    Secret(String),
    /// The name of a profile parameter. The compiler resolves it for the target (ADR-0047).
    Parameter(String),
}

/// One environment variable of a script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptEnvRef {
    pub name: String,
    pub value: ScriptEnvValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRef {
    pub name: String,
    pub interpreter: ScriptInterpreter,
    pub phase: HookPhase,
    pub body: String,
    /// Zero means the default limit of the control plane.
    pub timeout_seconds: u32,
    pub failure_policy: FailurePolicy,
    pub env: Vec<ScriptEnvRef>,
}

/// What the target agent reports about the trust mode (FR-HK-08).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerScripts {
    pub enabled: bool,
    pub interpreters: Vec<ScriptInterpreter>,
}

/// Checks one script against the facts of the target agent (FR-HK-10).
///
/// # Errors
///
/// Returns [`CompileError::ServerScriptsDisabled`] when the host did not turn on the trust mode,
/// and [`CompileError::InterpreterUnavailable`] when the host lacks the interpreter.
pub fn check_script(
    index: usize,
    script: &ScriptRef,
    target: &TargetFacts,
) -> Result<(), CompileError> {
    if !target.server_scripts.enabled {
        return Err(CompileError::ServerScriptsDisabled {
            field: FieldPath::new(format!("scripts[{index}]")),
        });
    }
    if !target
        .server_scripts
        .interpreters
        .contains(&script.interpreter)
    {
        return Err(CompileError::InterpreterUnavailable {
            field: FieldPath::new(format!("scripts[{index}].interpreter")),
            interpreter: script.interpreter,
        });
    }
    Ok(())
}

/// The step of one script, with the value of each variable resolved for the target (FR-PF-10).
///
/// # Errors
///
/// Returns [`CompileError::MissingParameter`] when a variable names a parameter that no source
/// gives a value.
pub(crate) fn script_step(
    index: usize,
    script: &ScriptRef,
    target: &TargetFacts,
) -> Result<Step, CompileError> {
    let mut env = Vec::with_capacity(script.env.len());
    for (position, variable) in script.env.iter().enumerate() {
        let value = match &variable.value {
            ScriptEnvValue::Plain(value) => script_env::Value::StringValue(value.clone()),
            ScriptEnvValue::Secret(id) => script_env::Value::SecretValue(id.clone()),
            // A text parameter reaches the agent as a value that it masks, because a profile may
            // hold a credential in a text value (FR-PF-12). A secret parameter keeps its reference,
            // so the job builder resolves it and writes `secret_delivered` (ADR-0017).
            ScriptEnvValue::Parameter(name) => match target.parameters.get(name) {
                Some(ParameterValue::Text(value)) => script_env::Value::MaskedValue(value.clone()),
                Some(ParameterValue::Secret(id)) => script_env::Value::SecretValue(id.clone()),
                None => {
                    return Err(CompileError::MissingParameter {
                        field: FieldPath::new(format!("scripts[{index}].env[{position}].value")),
                        name: name.clone(),
                    });
                }
            },
        };
        env.push(ScriptEnv {
            name: variable.name.clone(),
            value: Some(value),
        });
    }
    Ok(Step {
        step_id: format!("script-{index}"),
        timeout_seconds: script.timeout_seconds,
        kind: Some(step::Kind::Script(ScriptStep {
            name: script.name.clone(),
            interpreter: script.interpreter.to_proto() as i32,
            phase: phase_flag(script.phase),
            body: script.body.clone(),
            failure_policy: match script.failure_policy {
                FailurePolicy::AbortBackup => ProtoPolicy::AbortBackup as i32,
                FailurePolicy::Continue => ProtoPolicy::Continue as i32,
            },
            env,
        })),
    })
}

fn phase_flag(phase: HookPhase) -> i32 {
    match phase {
        HookPhase::Before => ProtoHookPhase::Before as i32,
        HookPhase::After => ProtoHookPhase::After as i32,
        HookPhase::OnSuccess => ProtoHookPhase::OnSuccess as i32,
        HookPhase::OnError => ProtoHookPhase::OnError as i32,
        HookPhase::Finally => ProtoHookPhase::Finally as i32,
    }
}
