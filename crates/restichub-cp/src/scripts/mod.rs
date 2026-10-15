//! Server scripts: the typed script of a profile, its checks and the trust mode of the agents
//! (FR-HK-08 to FR-HK-12, ADR-0037).
//!
//! A script is code that runs as root or SYSTEM on a host that turned on the local trust mode. The
//! control plane stores the body, the interpreter and the typed environment variables with the
//! profile. A secret variable holds the identifier of a secret of the purpose `hook_secret`. The
//! job builder replaces the identifier with the value when it sends the job and writes
//! `secret_delivered` (ADR-0017). Only an Admin changes scripts and templates, and every change
//! writes the SHA-256 hash of the body in the audit log (threat T33).

mod builtin;
mod templates;

use std::collections::BTreeSet;

use restichub_compiler::{
    FailurePolicy as CompilerPolicy, HookPhase as CompilerPhase, ScriptEnvRef, ScriptEnvValue,
    ScriptInterpreter as CompilerInterpreter, ScriptRef,
};
use restichub_db::ServerScriptsState;
use restichub_db::agents::Agent;
use restichub_proto::AgentCapability;
use restichub_proto::agent::v1::{AgentHello, ScriptInterpreter as ProtoInterpreter};
use restichub_proto::validate::{
    MAX_SCRIPT_BODY_BYTES, MAX_SCRIPT_ENV, MAX_SCRIPT_ENV_VALUE_BYTES, MAX_SCRIPT_NAME_CHARS,
    has_forbidden, is_script_env_name,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::profiles::HookPhase;
use crate::validation::Reason;

pub use templates::*;

/// The most scripts in one profile.
pub const MAX_SCRIPTS: usize = 32;

/// The most body bytes of all scripts in one profile. The signed job must fit in a message.
pub const MAX_TOTAL_BODY_BYTES: usize = 1024 * 1024;

/// The longest time limit of a script in seconds.
pub const MAX_TIMEOUT_SECONDS: u32 = 86_400;

/// The program that runs a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Interpreter {
    Powershell,
    Cmd,
    Bash,
    Sh,
}

impl Interpreter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Powershell => "powershell",
            Self::Cmd => "cmd",
            Self::Bash => "bash",
            Self::Sh => "sh",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "powershell" => Some(Self::Powershell),
            "cmd" => Some(Self::Cmd),
            "bash" => Some(Self::Bash),
            "sh" => Some(Self::Sh),
            _ => None,
        }
    }

    fn to_compiler(self) -> CompilerInterpreter {
        match self {
            Self::Powershell => CompilerInterpreter::Powershell,
            Self::Cmd => CompilerInterpreter::Cmd,
            Self::Bash => CompilerInterpreter::Bash,
            Self::Sh => CompilerInterpreter::Sh,
        }
    }
}

impl From<CompilerInterpreter> for Interpreter {
    fn from(value: CompilerInterpreter) -> Self {
        match value {
            CompilerInterpreter::Powershell => Self::Powershell,
            CompilerInterpreter::Cmd => Self::Cmd,
            CompilerInterpreter::Bash => Self::Bash,
            CompilerInterpreter::Sh => Self::Sh,
        }
    }
}

/// What a failed script does to the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    /// A failed `before` script stops the backup.
    AbortBackup,
    /// The job goes on and ends with warnings.
    Continue,
}

impl FailurePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AbortBackup => "abort_backup",
            Self::Continue => "continue",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "abort_backup" => Some(Self::AbortBackup),
            "continue" => Some(Self::Continue),
            _ => None,
        }
    }
}

/// The kind of a script variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvKind {
    /// The value is plain text.
    String,
    /// The value is the identifier of a secret of the purpose `hook_secret`.
    SecretRef,
}

/// One environment variable of a script. The agent gives it to the script as an environment
/// variable and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptEnvDef {
    /// Letters, digits and underscore, not first a digit.
    pub name: String,
    pub kind: EnvKind,
    /// The text, or the identifier of the secret.
    pub value: String,
}

/// A script of a profile or of a job template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptDef {
    /// The name in the job log. 1 to 128 characters.
    pub name: String,
    pub interpreter: Interpreter,
    pub phase: HookPhase,
    /// The text of the script. At most 65536 bytes.
    pub body: String,
    /// Zero means the default limit of one hour.
    #[serde(default)]
    pub timeout_seconds: u32,
    pub failure_policy: FailurePolicy,
    #[serde(default)]
    pub env: Vec<ScriptEnvDef>,
    /// The identifier of the template that the script came from. It has no effect on the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
}

/// The SHA-256 hash of a script body in lower-case hexadecimal (FR-HK-12).
#[must_use]
pub fn body_hash(body: &str) -> String {
    Sha256::digest(body.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The scripts of a profile for an audit diff: names, hashes and the names of the variables. It
/// holds no body and no value (FR-HK-12).
#[must_use]
pub fn audit_value(scripts: &[ScriptDef]) -> Value {
    Value::Array(
        scripts
            .iter()
            .map(|script| {
                json!({
                    "name": script.name,
                    "interpreter": script.interpreter.as_str(),
                    "phase": phase_name(script.phase),
                    "timeout_seconds": script.timeout_seconds,
                    "failure_policy": script.failure_policy.as_str(),
                    "sha256": body_hash(&script.body),
                    "env": script.env.iter().map(|variable| json!({
                        "name": variable.name,
                        "kind": match variable.kind {
                            EnvKind::String => "string",
                            EnvKind::SecretRef => "secret_ref",
                        },
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// The stored scripts of a profile row. A row that does not parse is an internal error.
pub(crate) fn from_row(value: &Value) -> Result<Vec<ScriptDef>, ApiError> {
    serde_json::from_value(value.clone())
        .map_err(|error| ApiError::internal(format!("the stored scripts are not valid: {error}")))
}

pub(crate) fn phase_name(phase: HookPhase) -> &'static str {
    match phase {
        HookPhase::Before => "before",
        HookPhase::After => "after",
        HookPhase::OnSuccess => "on_success",
        HookPhase::OnError => "on_error",
        HookPhase::Finally => "finally",
    }
}

pub(crate) fn parse_phase(value: &str) -> Option<HookPhase> {
    match value {
        "before" => Some(HookPhase::Before),
        "after" => Some(HookPhase::After),
        "on_success" => Some(HookPhase::OnSuccess),
        "on_error" => Some(HookPhase::OnError),
        "finally" => Some(HookPhase::Finally),
        _ => None,
    }
}

/// The compiler input of one script.
#[must_use]
pub fn compiler_script(script: &ScriptDef) -> ScriptRef {
    ScriptRef {
        name: script.name.clone(),
        interpreter: script.interpreter.to_compiler(),
        phase: match script.phase {
            HookPhase::Before => CompilerPhase::Before,
            HookPhase::After => CompilerPhase::After,
            HookPhase::OnSuccess => CompilerPhase::OnSuccess,
            HookPhase::OnError => CompilerPhase::OnError,
            HookPhase::Finally => CompilerPhase::Finally,
        },
        body: script.body.clone(),
        timeout_seconds: script.timeout_seconds,
        failure_policy: match script.failure_policy {
            FailurePolicy::AbortBackup => CompilerPolicy::AbortBackup,
            FailurePolicy::Continue => CompilerPolicy::Continue,
        },
        env: script
            .env
            .iter()
            .map(|variable| ScriptEnvRef {
                name: variable.name.clone(),
                value: match variable.kind {
                    EnvKind::String => ScriptEnvValue::Plain(variable.value.clone()),
                    EnvKind::SecretRef => ScriptEnvValue::Secret(variable.value.clone()),
                },
            })
            .collect(),
    }
}

/// Validates the scripts of a profile or of a job template and returns them normalized.
///
/// # Errors
///
/// Returns a field error such as `scripts[1].body` for the first rule that a script breaks.
pub fn validate_scripts(scripts: Vec<ScriptDef>, field: &str) -> Result<Vec<ScriptDef>, ApiError> {
    if scripts.len() > MAX_SCRIPTS {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    let mut total = 0_usize;
    let mut result = Vec::with_capacity(scripts.len());
    for (index, script) in scripts.into_iter().enumerate() {
        total = total.saturating_add(script.body.len());
        if total > MAX_TOTAL_BODY_BYTES {
            return Err(ApiError::invalid_field(
                format!("{field}[{index}].body"),
                Reason::TooLong,
            ));
        }
        result.push(validate_script(script, &format!("{field}[{index}]"))?);
    }
    Ok(result)
}

/// Validates one script. `prefix` is the field path of the script, for example `scripts[0]`.
pub fn validate_script(mut script: ScriptDef, prefix: &str) -> Result<ScriptDef, ApiError> {
    let at = |name: &str| format!("{prefix}.{name}");
    script.name = script.name.trim().to_owned();
    if script.name.is_empty() {
        return Err(ApiError::invalid_field(at("name"), Reason::Empty));
    }
    if script.name.chars().count() > MAX_SCRIPT_NAME_CHARS {
        return Err(ApiError::invalid_field(at("name"), Reason::TooLong));
    }
    if has_forbidden(&script.name) {
        return Err(ApiError::invalid_field(
            at("name"),
            Reason::InvalidCharacter,
        ));
    }
    if script.body.trim().is_empty() {
        return Err(ApiError::invalid_field(at("body"), Reason::Empty));
    }
    if script.body.len() > MAX_SCRIPT_BODY_BYTES {
        return Err(ApiError::invalid_field(at("body"), Reason::TooLong));
    }
    if script.body.contains('\0') {
        return Err(ApiError::invalid_field(
            at("body"),
            Reason::InvalidCharacter,
        ));
    }
    if script.timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(ApiError::invalid_field(
            at("timeout_seconds"),
            Reason::TooLong,
        ));
    }
    if script.env.len() > MAX_SCRIPT_ENV {
        return Err(ApiError::invalid_field(at("env"), Reason::TooLong));
    }
    let mut names = BTreeSet::new();
    for (index, variable) in script.env.iter().enumerate() {
        let field = format!("{prefix}.env[{index}]");
        if !is_script_env_name(&variable.name) {
            return Err(ApiError::invalid_field(
                format!("{field}.name"),
                Reason::InvalidCharacter,
            ));
        }
        if !names.insert(variable.name.to_ascii_uppercase()) {
            return Err(ApiError::invalid_field(
                format!("{field}.name"),
                Reason::Reserved,
            ));
        }
        if variable.value.len() > MAX_SCRIPT_ENV_VALUE_BYTES {
            return Err(ApiError::invalid_field(
                format!("{field}.value"),
                Reason::TooLong,
            ));
        }
        match variable.kind {
            EnvKind::String if has_forbidden(&variable.value) => {
                return Err(ApiError::invalid_field(
                    format!("{field}.value"),
                    Reason::InvalidCharacter,
                ));
            }
            EnvKind::SecretRef if uuid::Uuid::parse_str(&variable.value).is_err() => {
                return Err(ApiError::invalid_field(
                    format!("{field}.value"),
                    Reason::InvalidFormat,
                ));
            }
            _ => {}
        }
    }
    if let Some(template_id) = &script.template_id
        && (template_id.is_empty() || template_id.len() > 64 || has_forbidden(template_id))
    {
        return Err(ApiError::invalid_field(
            at("template_id"),
            Reason::InvalidFormat,
        ));
    }
    Ok(script)
}

/// The trust mode that a hello reports. An agent without the capability `scripts-v1` reports none,
/// and the control plane treats it as disabled (FR-HK-08).
pub(crate) struct HelloScripts {
    pub(crate) state: ServerScriptsState,
    pub(crate) interpreters: Vec<String>,
}

/// Reads the trust mode of a hello.
pub(crate) fn from_hello(hello: &AgentHello, capabilities: &[AgentCapability]) -> HelloScripts {
    use restichub_proto::agent::v1::ServerScriptsState as Proto;
    if !capabilities.contains(&AgentCapability::ScriptsV1) {
        return HelloScripts {
            state: ServerScriptsState::Unknown,
            interpreters: Vec::new(),
        };
    }
    let state = match Proto::try_from(hello.server_scripts_state) {
        Ok(Proto::Enabled) => ServerScriptsState::Enabled,
        Ok(Proto::Insecure) => ServerScriptsState::Insecure,
        Ok(Proto::Disabled) => ServerScriptsState::Disabled,
        Ok(Proto::Unspecified) | Err(_) => ServerScriptsState::Unknown,
    };
    let mut interpreters = Vec::new();
    // Only a host that trusts scripts lists its interpreters.
    if state == ServerScriptsState::Enabled {
        for value in &hello.script_interpreters {
            let known = ProtoInterpreter::try_from(*value)
                .ok()
                .and_then(CompilerInterpreter::from_proto);
            if let Some(known) = known {
                let name = Interpreter::from(known).as_str().to_owned();
                if !interpreters.contains(&name) {
                    interpreters.push(name);
                }
            }
        }
    }
    HelloScripts {
        state,
        interpreters,
    }
}

/// The trust facts of an agent for the compiler (FR-HK-08).
#[must_use]
pub fn server_scripts_of(agent: &Agent) -> restichub_compiler::ServerScripts {
    restichub_compiler::ServerScripts {
        enabled: agent.server_scripts == ServerScriptsState::Enabled,
        interpreters: agent
            .script_interpreters
            .iter()
            .filter_map(|name| Interpreter::parse(name))
            .map(Interpreter::to_compiler)
            .collect(),
    }
}
