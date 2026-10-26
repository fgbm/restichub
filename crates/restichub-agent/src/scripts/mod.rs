//! Scripts from the control plane on a host with the local trust mode (FR-HK-08, FR-HK-09,
//! ADR-0037).
//!
//! The agent runs the code of a script step as root or LocalSystem, so only the host administrator
//! decides whether it does. The decision is a small file next to the configuration directory, with
//! the same ownership rule as the other trusted paths ([`crate::trust`], AC-HK-08). Without the
//! file, or when another account can write it, the mode is off. A job, the control plane and a
//! self-update cannot change the file.
//!
//! A script goes to a new file in a protected directory of the cache. The interpreter starts with
//! fixed flags, the environment of the service without the restic variables and the variables of
//! the step, and the directory is removed when the run ends (AC-HK-09). The search path keeps only
//! directories that the host account controls. The values reach the script only as environment
//! variables.

mod interpreter;
mod search_path;
mod setting;
#[cfg(test)]
mod tests;
#[cfg(target_os = "linux")]
pub(crate) mod transient;

use std::path::{Path, PathBuf};
use std::time::Duration;

use restichub_proto::agent::v1::{
    RejectReason, ScriptInterpreter, ScriptStep, ServerScriptsState, script_env,
};
use zeroize::Zeroizing;

use crate::exec::Invocation;
use crate::trust::Trust;

#[cfg(windows)]
pub(crate) use setting::allow_list_file_for;
pub use setting::{ScriptMode, mode_file, mode_file_for, removed_allow_list_file, set_mode};

/// The time limit of a script whose step names none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3600);

pub const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

/// Why the agent cannot run a script step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptRefusal {
    /// The host did not turn on the trust mode, or the setting file is not trusted (AC-HK-07).
    Disabled,
    /// The step breaks a rule, or the host lacks the interpreter. The text names no value.
    Invalid(String),
    Unavailable(String),
}

impl ScriptRefusal {
    /// The reason that the agent sends in `JobRejected`.
    #[must_use]
    pub fn reject_reason(&self) -> RejectReason {
        match self {
            Self::Disabled => RejectReason::ServerScriptsDisabled,
            Self::Invalid(_) => RejectReason::InvalidScript,
            Self::Unavailable(_) => RejectReason::Internal,
        }
    }
}

/// How the agent runs a script step (ADR-0053).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptRunner {
    /// A child of the agent, inside the sandbox of its unit. Windows, macOS and a Linux host
    /// without systemd use it.
    Direct,
    /// A transient systemd unit without the sandbox of the agent unit.
    Systemd,
}

impl ScriptRunner {
    /// The runner of this host: systemd on Linux where the launcher exists.
    #[must_use]
    pub fn for_host() -> Self {
        #[cfg(target_os = "linux")]
        if transient::available() {
            return Self::Systemd;
        }
        Self::Direct
    }
}

/// The transient unit of one script step (ADR-0053).
#[derive(Debug, Clone)]
pub struct TransientUnit {
    pub name: String,
    pub env_file: PathBuf,
    /// The backstop after the timeout of the step, so a unit that no cancel reaches ends.
    pub runtime: Duration,
}

/// The place of the setting file, the trust rule that applies to it and the directory for the
/// script files.
#[derive(Debug, Clone)]
pub struct ScriptStore {
    path: PathBuf,
    trust: Trust,
    scratch_root: PathBuf,
    search_path: Option<std::ffi::OsString>,
    runner: ScriptRunner,
}

impl ScriptStore {
    /// The store of this host: the file next to the configuration directory.
    #[must_use]
    pub fn for_config_dir(config_dir: &Path, cache: &Path) -> Self {
        Self {
            path: mode_file_for(config_dir),
            trust: Trust::Host,
            scratch_root: cache.join("scripts"),
            search_path: None,
            runner: ScriptRunner::for_host(),
        }
    }

    /// A store with a chosen file and trust rule, for a test.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn new(path: PathBuf, trust: Trust, cache: &Path) -> Self {
        Self {
            path,
            trust,
            scratch_root: cache.join("scripts"),
            search_path: None,
            runner: ScriptRunner::Direct,
        }
    }

    /// Chooses the runner of a script step, for a test.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn with_runner(mut self, runner: ScriptRunner) -> Self {
        self.runner = runner;
        self
    }

    /// Removes the script directories that a crash left behind. A new agent has no running job, so
    /// every directory under the root is a leftover (AC-HK-09).
    pub fn clean_leftovers(&self) {
        if self.runner == ScriptRunner::Systemd {
            #[cfg(target_os = "linux")]
            transient::stop_leftovers();
        }
        match std::fs::remove_dir_all(&self.scratch_root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(%error, "cannot remove the old script directories"),
        }
    }

    /// Filters this search path instead of the one of the process, for a test.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn with_search_path(mut self, path: std::ffi::OsString) -> Self {
        self.search_path = Some(path);
        self
    }

    /// Reads the setting file now (AC-HK-08).
    #[must_use]
    pub fn mode(&self) -> ScriptMode {
        setting::read(&self.path, self.trust)
    }

    /// The state and the interpreters for `AgentHello`. A host that did not turn on the mode
    /// reports no interpreter (FR-HK-08).
    #[must_use]
    pub fn report(&self) -> (ServerScriptsState, Vec<ScriptInterpreter>) {
        let mode = self.mode();
        let interpreters = if mode == ScriptMode::Enabled {
            interpreter::available(self.trust)
        } else {
            Vec::new()
        };
        (mode.to_proto(), interpreters)
    }

    /// Checks a step before the agent acknowledges the job: the mode, the rules of the step and
    /// the interpreter (AC-HK-07).
    ///
    /// # Errors
    ///
    /// Returns the reason that the agent refuses the step.
    pub fn check(&self, step: &ScriptStep) -> Result<(), ScriptRefusal> {
        if self.mode() != ScriptMode::Enabled {
            return Err(ScriptRefusal::Disabled);
        }
        validate(step)?;
        let kind = interpreter_of(step)?;
        interpreter::locate(kind, self.trust)
            .map(|_| ())
            .ok_or_else(|| ScriptRefusal::Invalid("the host has no such interpreter".to_owned()))
    }

    /// Checks the mode again and writes the script file. The returned value removes the file and
    /// its directory when it is dropped (AC-HK-09).
    ///
    /// # Errors
    ///
    /// Returns the reason that the agent cannot run the step.
    pub fn prepare(
        &self,
        step: &ScriptStep,
        timeout_seconds: u32,
    ) -> Result<PreparedScript, ScriptRefusal> {
        self.check(step)?;
        let kind = interpreter_of(step)?;
        let program = interpreter::locate(kind, self.trust)
            .ok_or_else(|| ScriptRefusal::Invalid("the host has no such interpreter".to_owned()))?;
        let scratch = Scratch::create(&self.scratch_root)
            .map_err(|error| ScriptRefusal::Unavailable(format!("{error:#}")))?;
        let file = scratch.path().join(interpreter::file_name(kind));
        crate::paths::create_protected_file(&file, &interpreter::file_bytes(kind, &step.body))
            .map_err(|error| ScriptRefusal::Unavailable(format!("{error:#}")))?;

        let mut env = Vec::new();
        let mut secrets = Vec::new();
        for variable in &step.env {
            let (value, secret) = match &variable.value {
                Some(script_env::Value::StringValue(value)) => (value, false),
                // A masked value is a text parameter of the profile: it is no secret, but the
                // profile may hold a credential, so the scrubber hides it too (FR-PF-12).
                Some(
                    script_env::Value::SecretValue(value) | script_env::Value::MaskedValue(value),
                ) => (value, true),
                None => return Err(ScriptRefusal::Invalid("a variable has no value".to_owned())),
            };
            env.push((variable.name.clone(), Zeroizing::new(value.clone())));
            if secret && !value.is_empty() {
                secrets.push(Zeroizing::new(value.clone()));
            }
        }
        let timeout = if timeout_seconds == 0 {
            DEFAULT_TIMEOUT
        } else {
            Duration::from_secs(timeout_seconds.into()).min(MAX_TIMEOUT)
        };
        let unit = self.transient_unit(&scratch, &env, timeout)?;
        let service_path = self
            .search_path
            .clone()
            .or_else(|| std::env::var_os("PATH"));
        let search = search_path::trusted(service_path.as_deref(), self.trust);
        let dropped_path =
            (!search.dropped.is_empty()).then(|| search_path::dropped_text(&search.dropped));
        Ok(PreparedScript {
            dropped_path,
            invocation: Invocation {
                program,
                args: interpreter::arguments(kind, &file),
                env,
                working_dir: scratch.path().to_path_buf(),
                timeout,
                secrets,
                search_path: Some(search.value),
                unit,
            },
            scratch: ScratchGuard { _scratch: scratch },
        })
    }

    /// Writes the parameter file for a transient unit; the direct runner has none (ADR-0053).
    fn transient_unit(
        &self,
        scratch: &Scratch,
        env: &[(String, Zeroizing<String>)],
        timeout: Duration,
    ) -> Result<Option<TransientUnit>, ScriptRefusal> {
        if self.runner != ScriptRunner::Systemd {
            return Ok(None);
        }
        #[cfg(target_os = "linux")]
        {
            let env_file = scratch.path().join("environment.sh");
            crate::paths::create_protected_file(
                &env_file,
                transient::environment_file(env).as_bytes(),
            )
            .map_err(|error| ScriptRefusal::Unavailable(format!("{error:#}")))?;
            Ok(Some(TransientUnit {
                name: transient::unit_name(scratch.random()),
                env_file,
                runtime: transient::runtime_backstop(timeout),
            }))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (scratch, env, timeout);
            Ok(None)
        }
    }
}

/// A script that is ready to start. Dropping it removes the script file.
pub struct PreparedScript {
    pub invocation: Invocation,
    /// The log line about the search path entries that the agent dropped, or `None` when it
    /// dropped none.
    pub dropped_path: Option<String>,
    pub scratch: ScratchGuard,
}

/// Removes the script file and its directory when it is dropped (AC-HK-09).
pub struct ScratchGuard {
    _scratch: Scratch,
}

fn interpreter_of(step: &ScriptStep) -> Result<ScriptInterpreter, ScriptRefusal> {
    match ScriptInterpreter::try_from(step.interpreter) {
        Ok(ScriptInterpreter::Unspecified) | Err(_) => Err(ScriptRefusal::Invalid(
            "the interpreter is unknown".to_owned(),
        )),
        Ok(kind) => Ok(kind),
    }
}

/// The rules of the protocol again, because the agent trusts no earlier check (FR-HK-09).
fn validate(step: &ScriptStep) -> Result<(), ScriptRefusal> {
    use restichub_proto::validate::{MAX_SCRIPT_BODY_BYTES, is_script_env_name};
    let invalid = |text: &str| Err(ScriptRefusal::Invalid(text.to_owned()));
    if step.body.trim().is_empty() || step.body.len() > MAX_SCRIPT_BODY_BYTES {
        return invalid("the script body is empty or too long");
    }
    for variable in &step.env {
        if !is_script_env_name(&variable.name) {
            return invalid("a variable name is not allowed");
        }
        // An environment variable is a NUL-terminated string, and the shell cannot hold the byte.
        if matches!(
            &variable.value,
            Some(
                script_env::Value::StringValue(value)
                    | script_env::Value::SecretValue(value)
                    | script_env::Value::MaskedValue(value)
            ) if value.contains('\0')
        ) {
            return invalid("a variable value contains a NUL byte");
        }
    }
    Ok(())
}

/// A new directory for one script run. Only the service account can use it.
pub(crate) struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn create(root: &Path) -> anyhow::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("cannot read random bytes: {error}"))?;
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        crate::paths::create_protected_dir(root)?;
        let path = root.join(name);
        crate::paths::create_protected_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// The random part of the directory name, which also names the transient unit (ADR-0053).
    fn random(&self) -> &str {
        self.path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("script")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            tracing::warn!(%error, "cannot remove the script directory");
        }
    }
}
