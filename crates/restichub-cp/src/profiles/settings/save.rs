//! The save-time compile of a profile (FR-PF-05, AC-PF-02).
//!
//! A defect that no target accepts refuses the request. A defect of one operating system or of an
//! older restic is a warning, because one profile can serve several hosts.

use std::collections::BTreeMap;

use restichub_compiler::{
    Architecture, CompileError, OperatingSystem, ResticVersion, TargetFacts, compile,
};

use crate::error::ApiError;
use crate::parameters::ParameterDef;
use crate::validation::Reason;

use super::{ProfileWarning, ValidatedProfile};

/// The outcome of one synthetic compile at save time (FR-PF-05, AC-PF-02).
enum SaveOutcome {
    /// The profile is wrong for every target: the request is refused.
    Refusal(ApiError),
    /// The profile is wrong for one target only: the caller gets a warning.
    Warning(ProfileWarning),
}

/// Runs the compiler against a synthetic target for each operating system (FR-PF-05).
///
/// A template error is wrong for every target, so it refuses the request. An operating-system or
/// version mismatch is a warning, because a profile may target several operating systems. The
/// synthetic target has no scripts: the scripts are checked against the report of the real agents
/// (FR-HK-10).
///
/// # Errors
///
/// Returns a field error for a defect that no target accepts.
pub fn save_warnings(profile: &ValidatedProfile) -> Result<Vec<ProfileWarning>, ApiError> {
    let mut compiler_profile = profile.to_compiler_profile();
    // Scripts depend on the trust mode of the target agents, which the synthetic target lacks.
    compiler_profile.scripts.clear();
    let mut warnings = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for os in [
        OperatingSystem::Linux,
        OperatingSystem::Windows,
        OperatingSystem::Macos,
    ] {
        let target = synthetic_target(os, &profile.parameters);
        if let Err(error) = compile(&compiler_profile, None, &target) {
            match classify_save_error(error) {
                SaveOutcome::Refusal(api) => return Err(api),
                SaveOutcome::Warning(warning) => {
                    let key = (
                        warning.field.clone(),
                        warning.reason.clone(),
                        warning.os.clone(),
                    );
                    if seen.insert(key) {
                        warnings.push(warning);
                    }
                }
            }
        }
    }
    Ok(warnings)
}

/// The target of the save-time compile, with a value for each declared parameter (FR-PF-05).
///
/// A text parameter takes its default or a placeholder; a placeholder path or tag satisfies the
/// compile, and the owner rule for an optional parameter without a default runs before this check.
fn synthetic_target(os: OperatingSystem, parameters: &[ParameterDef]) -> TargetFacts {
    TargetFacts {
        agent_id: "synthetic".to_owned(),
        agent_name: "agent".to_owned(),
        hostname: "host".to_owned(),
        os,
        arch: Architecture::X86_64,
        labels: BTreeMap::new(),
        parameters: parameters
            .iter()
            .map(|definition| {
                let value = match definition.kind {
                    restichub_db::ParameterKind::Text => restichub_compiler::ParameterValue::Text(
                        definition
                            .default_value
                            .clone()
                            .unwrap_or_else(|| "value".to_owned()),
                    ),
                    restichub_db::ParameterKind::Secret => {
                        restichub_compiler::ParameterValue::Secret("secret".to_owned())
                    }
                };
                (definition.name.clone(), value)
            })
            .collect(),
        // The newest supported version: the option and version check runs in the preview against
        // the real agent (FR-PF-05, AC-AG-09).
        restic_version: ResticVersion {
            major: 0,
            minor: 19,
            patch: 0,
        },
        now: time::OffsetDateTime::now_utc(),
        server_scripts: restichub_compiler::ServerScripts::default(),
    }
}

fn classify_save_error(error: CompileError) -> SaveOutcome {
    let warning = |field: &str, reason: &str, os: Option<String>| {
        SaveOutcome::Warning(ProfileWarning {
            field: field.to_owned(),
            reason: reason.to_owned(),
            os,
        })
    };
    match error {
        CompileError::UnknownVariable { field, .. }
        | CompileError::InvalidDate { field, .. }
        | CompileError::UnterminatedTemplate { field }
        | CompileError::InvalidValue { field } => SaveOutcome::Refusal(ApiError::invalid_field(
            field.as_str(),
            Reason::InvalidFormat,
        )),
        CompileError::ServerScriptsDisabled { field }
        | CompileError::InterpreterUnavailable { field, .. } => {
            SaveOutcome::Refusal(ApiError::invalid_field(field.as_str(), Reason::Unavailable))
        }
        CompileError::StepsRejected { .. } => {
            SaveOutcome::Refusal(ApiError::invalid_field("steps", Reason::InvalidFormat))
        }
        CompileError::MissingParameter { field, .. } => SaveOutcome::Refusal(ApiError::parameters(
            field.as_str(),
            Reason::ParameterMissing,
            Vec::new(),
        )),
        CompileError::SecretParameterInTemplate { field, .. } => SaveOutcome::Refusal(
            ApiError::invalid_field(field.as_str(), Reason::ParameterSecretInTemplate),
        ),
        CompileError::ParameterNotAllowed { field, .. } => SaveOutcome::Refusal(
            ApiError::invalid_field(field.as_str(), Reason::ParameterNotAllowed),
        ),
        CompileError::MissingLabel { field, .. } => warning(field.as_str(), "unknown", None),
        CompileError::UnsupportedOs { field, os, .. } => {
            warning(field.as_str(), "unsupported", Some(os.as_str().to_owned()))
        }
        CompileError::NoPaths { os } => warning("paths", "empty", Some(os.as_str().to_owned())),
        CompileError::IncompatibleOption { field, .. } => {
            warning(field.as_str(), "incompatible", None)
        }
        CompileError::UnsupportedRestic { .. } => warning("agent_id", "incompatible", None),
        CompileError::InvalidCron { .. } => warning("cron", "invalid_format", None),
    }
}
