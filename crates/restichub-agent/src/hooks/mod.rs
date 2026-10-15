//! The hook allow-list of the agent (FR-HK-01 to FR-HK-05, ADR-0004).
//!
//! A hook is a program that a host administrator lists in a protected file next to the agent
//! configuration. The control plane refers to a hook by its identifier and sends typed parameter
//! values. The agent checks the values against the schema of the hook and starts the program
//! without a shell, with a fixed argument list, a cleared environment and no input.
//!
//! The agent trusts the file only when root or Administrators own it and nobody else can write it
//! or a directory above it ([`trust`], AC-HK-01). It reads the file again at each job, so a change
//! applies at the next job and a tampered file stops every hook at once. A job, the control plane
//! and a self-update cannot write the file: a restore refuses to touch it
//! ([`crate::restore_roots`], threat T2).

mod file;
pub mod trust;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use restichub_compiler::hook::phase_of;
use restichub_compiler::{
    AllowListedHook, HookParameterSpec, HookPhase, HookValue, ParameterKind, ValueError,
};
use restichub_proto::agent::v1::{HookAllowListState, HookMetadata, HookStep, RejectReason};
use zeroize::Zeroizing;

pub use trust::Trust;

/// The file with the allow-list, next to the configuration directory like the restore roots file,
/// so a swap of the identity never removes it.
#[must_use]
pub fn allow_list_file() -> PathBuf {
    allow_list_file_for(&crate::paths::config_dir())
}

/// The allow-list file that belongs to a configuration directory.
#[must_use]
pub fn allow_list_file_for(config_dir: &Path) -> PathBuf {
    let name = config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("restichub-agent");
    config_dir.with_file_name(format!("{name}-hooks.toml"))
}

/// How a parameter reaches the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mapping {
    /// As the argument `--name=value`. A boolean is `--name` when true and absent when false.
    Flag,
    /// As one argument at a fixed position after the flags.
    Positional(u32),
    /// In the named environment variable. Only a secret reference uses it.
    Env(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Parameter {
    pub(crate) name: String,
    pub(crate) spec: HookParameterSpec,
    pub(crate) mapping: Mapping,
}

/// A validated entry of the file with its fixed command line and schema.
#[derive(Debug, Clone)]
pub struct Hook {
    id: String,
    executable: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    working_dir: PathBuf,
    env: Vec<(String, String)>,
    phases: Vec<HookPhase>,
    parameters: Vec<Parameter>,
}

/// Why the agent refuses the parameters of a hook step (FR-HK-03).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvocationError {
    #[error("the hook does not run in this phase")]
    PhaseNotAllowed,
    #[error("the step names a parameter that the hook does not have")]
    UnknownParameter,
    #[error("the step repeats a parameter")]
    DuplicateParameter,
    #[error("the step lacks a required parameter")]
    MissingParameter,
    #[error("a value does not obey the schema: {0}")]
    InvalidValue(ValueError),
    #[error("a positional value starts with '-'")]
    LeadingDash,
}

/// Everything that the agent needs to start one hook. It holds the secret values, so it has no
/// derived `Debug`.
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, Zeroizing<String>)>,
    pub working_dir: PathBuf,
    pub timeout: Duration,
    /// Secret values that the scrubber must remove from the hook output.
    pub secrets: Vec<Zeroizing<String>>,
}

impl std::fmt::Debug for Invocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Invocation")
            .field("program", &self.program)
            .field("args", &self.args)
            .field(
                "env",
                &self.env.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            )
            .field("working_dir", &self.working_dir)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Hook {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// The part of the schema that the agent reports to the control plane. It has no path.
    #[must_use]
    pub fn metadata(&self) -> AllowListedHook {
        AllowListedHook {
            parameters: self
                .parameters
                .iter()
                .map(|parameter| (parameter.name.clone(), parameter.spec.clone()))
                .collect(),
            phases: self.phases.clone(),
        }
    }

    /// Checks the values of a step against the schema and builds the process invocation
    /// (FR-HK-03, AC-HK-04).
    ///
    /// Each value becomes exactly one argument or one environment variable. The agent builds no
    /// string that a shell reads.
    ///
    /// # Errors
    ///
    /// Returns the first rule that the step breaks, as an [`InvocationError`].
    pub fn invocation(&self, step: &HookStep) -> Result<Invocation, InvocationError> {
        let phase = phase_of(step.phase).ok_or(InvocationError::PhaseNotAllowed)?;
        if !self.phases.is_empty() && !self.phases.contains(&phase) {
            return Err(InvocationError::PhaseNotAllowed);
        }
        let mut values: BTreeMap<&str, HookValue> = BTreeMap::new();
        for parameter in &step.parameters {
            let Some(known) = self
                .parameters
                .iter()
                .find(|known| known.name == parameter.name)
            else {
                return Err(InvocationError::UnknownParameter);
            };
            let typed = parameter
                .typed
                .as_ref()
                .ok_or(InvocationError::MissingParameter)?;
            let value = HookValue::from_proto(typed);
            known
                .spec
                .check(&value)
                .map_err(InvocationError::InvalidValue)?;
            if values.insert(known.name.as_str(), value).is_some() {
                return Err(InvocationError::DuplicateParameter);
            }
        }
        if self.parameters.iter().any(|parameter| {
            parameter.spec.required && !values.contains_key(parameter.name.as_str())
        }) {
            return Err(InvocationError::MissingParameter);
        }

        let mut args = self.args.clone();
        let mut env: Vec<(String, Zeroizing<String>)> = self
            .env
            .iter()
            .map(|(name, value)| (name.clone(), Zeroizing::new(value.clone())))
            .collect();
        let mut secrets = Vec::new();
        let mut positional: Vec<(u32, String)> = Vec::new();
        for parameter in &self.parameters {
            let Some(value) = values.get(parameter.name.as_str()) else {
                continue;
            };
            match (&parameter.mapping, value) {
                (Mapping::Env(name), HookValue::Secret(secret)) => {
                    env.push((name.clone(), Zeroizing::new(secret.clone())));
                    secrets.push(Zeroizing::new(secret.clone()));
                }
                (Mapping::Flag, HookValue::Boolean(true)) => {
                    args.push(format!("--{}", parameter.name));
                }
                (Mapping::Flag, HookValue::Boolean(false)) => {}
                (Mapping::Flag, value) => {
                    args.push(format!("--{}={}", parameter.name, text(value)));
                }
                (Mapping::Positional(position), value) => {
                    let text = text(value);
                    // A positional value that starts with '-' reads as an option in most programs.
                    if text.starts_with('-')
                        && !matches!(parameter.spec.kind, ParameterKind::Enum { .. })
                    {
                        return Err(InvocationError::LeadingDash);
                    }
                    positional.push((*position, text));
                }
                // The schema allows no other pair: a secret uses the environment only.
                (Mapping::Env(_), _) => return Err(InvocationError::UnknownParameter),
            }
        }
        positional.sort_by_key(|(position, _)| *position);
        args.extend(positional.into_iter().map(|(_, text)| text));

        Ok(Invocation {
            program: self.executable.clone(),
            args,
            env,
            working_dir: self.working_dir.clone(),
            timeout: self.timeout,
            secrets,
        })
    }
}

fn text(value: &HookValue) -> String {
    match value {
        HookValue::String(text) | HookValue::Secret(text) => text.clone(),
        HookValue::Integer(number) => number.to_string(),
        HookValue::Boolean(flag) => flag.to_string(),
    }
}

/// Whether the agent trusts the allow-list file (FR-HK-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowListState {
    /// No file. No hook runs.
    Absent,
    Loaded,
    /// Another account can change the file or a directory above it. No hook runs.
    Insecure,
    /// The file does not obey the format. No hook runs.
    Invalid,
}

impl AllowListState {
    #[must_use]
    pub fn to_proto(self) -> HookAllowListState {
        match self {
            Self::Absent => HookAllowListState::Absent,
            Self::Loaded => HookAllowListState::Loaded,
            Self::Insecure => HookAllowListState::Insecure,
            Self::Invalid => HookAllowListState::Invalid,
        }
    }
}

/// The hooks that one read of the file gave, with the state of the file.
#[derive(Debug, Clone)]
pub struct AllowList {
    state: AllowListState,
    hooks: BTreeMap<String, Hook>,
    trust: Trust,
}

/// Why the agent cannot run a hook of a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The file is not trusted or not valid, or the executable of the hook is not trusted.
    Insecure,
    /// No entry has the identifier of the step.
    Unknown,
    /// A parameter value breaks the schema of the hook.
    Parameters(InvocationError),
}

impl Refusal {
    /// The reason that the agent sends in `JobRejected`.
    #[must_use]
    pub fn reject_reason(&self) -> RejectReason {
        match self {
            Self::Insecure => RejectReason::AllowListInsecure,
            Self::Unknown => RejectReason::UnknownHook,
            Self::Parameters(_) => RejectReason::InvalidHookParameters,
        }
    }
}

impl AllowList {
    #[must_use]
    pub fn state(&self) -> AllowListState {
        self.state
    }

    /// The hooks that the agent can run now: the executable passes the trust check. A hook with
    /// an executable that another account can change is not listed.
    pub fn runnable(&self) -> impl Iterator<Item = &Hook> {
        self.hooks
            .values()
            .filter(|hook| trust::check_executable(&hook.executable, self.trust).is_ok())
    }

    /// The metadata for the `AgentHello` (FR-HK-04).
    #[must_use]
    pub fn report(&self) -> Vec<HookMetadata> {
        self.runnable()
            .map(|hook| hook.metadata().to_proto(&hook.id))
            .collect()
    }

    /// Finds the hook of a step and checks its executable and its parameters (AC-HK-01, AC-HK-03,
    /// AC-HK-04). The executable is checked at each call, so a file that changes after the
    /// admission of the job stops the hook.
    ///
    /// # Errors
    ///
    /// Returns the reason that the agent refuses the step.
    pub fn prepare(&self, step: &HookStep) -> Result<Invocation, Refusal> {
        if matches!(
            self.state,
            AllowListState::Insecure | AllowListState::Invalid
        ) {
            return Err(Refusal::Insecure);
        }
        let hook = self.hooks.get(&step.hook_id).ok_or(Refusal::Unknown)?;
        let resolved = trust::check_executable(&hook.executable, self.trust).map_err(|error| {
            tracing::warn!(hook = %hook.id, error = %format!("{error:#}"), "a hook executable is not trusted");
            Refusal::Insecure
        })?;
        let mut invocation = hook.invocation(step).map_err(Refusal::Parameters)?;
        // The resolved path names the file that the agent checked.
        invocation.program = resolved;
        Ok(invocation)
    }

    /// The executables of the hooks, which a restore may not overwrite (threat T2).
    #[must_use]
    pub fn executables(&self) -> Vec<PathBuf> {
        self.hooks
            .values()
            .map(|hook| hook.executable.clone())
            .collect()
    }
}

/// The place of the allow-list file and the trust rule that applies to it.
#[derive(Debug, Clone)]
pub struct HookStore {
    path: PathBuf,
    trust: Trust,
}

impl HookStore {
    /// The store of this host: the file next to the configuration directory.
    #[must_use]
    pub fn for_config_dir(config_dir: &Path) -> Self {
        Self {
            path: allow_list_file_for(config_dir),
            trust: Trust::Host,
        }
    }

    /// A store with a chosen file and trust rule, for a test.
    #[must_use]
    pub fn new(path: PathBuf, trust: Trust) -> Self {
        Self { path, trust }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the file. A file that is missing, not trusted or not valid gives an allow-list
    /// without hooks and the matching state (AC-HK-01).
    #[must_use]
    pub fn load(&self) -> AllowList {
        let empty = |state| AllowList {
            state,
            hooks: BTreeMap::new(),
            trust: self.trust,
        };
        match std::fs::symlink_metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return empty(AllowListState::Absent);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot read the hook allow-list");
                return empty(AllowListState::Insecure);
            }
        }
        if let Err(error) = trust::check_file(&self.path, self.trust) {
            tracing::warn!(error = %format!("{error:#}"), "the hook allow-list is not trusted, so no hook runs");
            return empty(AllowListState::Insecure);
        }
        let text = match read_bounded(&self.path) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "cannot read the hook allow-list");
                return empty(AllowListState::Invalid);
            }
        };
        match file::parse(&text) {
            Ok(hooks) => AllowList {
                state: AllowListState::Loaded,
                hooks,
                trust: self.trust,
            },
            Err(error) => {
                tracing::warn!(%error, "the hook allow-list is not valid, so no hook runs");
                empty(AllowListState::Invalid)
            }
        }
    }
}

fn read_bounded(path: &Path) -> anyhow::Result<String> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(file::MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > file::MAX_FILE_BYTES {
        anyhow::bail!("the file is larger than {} bytes", file::MAX_FILE_BYTES);
    }
    Ok(text)
}

#[cfg(all(test, unix))]
mod tests;
