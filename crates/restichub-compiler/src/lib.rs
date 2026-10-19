//! ResticHub profile compiler.
//!
//! The compiler converts a profile to a sequence of typed steps for an agent
//! (FR-PF-04, FR-PF-05). The input is a [`Profile`] and the [`TargetFacts`] of
//! one target agent. The output is a [`Plan`], and [`preview`] renders the plan
//! as command lines with masked secrets.
//!
//! Template variables expand into values only (FR-PF-03). The argument builder
//! in [`arg`] keeps every value out of an option position (ADR-0021). The agent
//! uses that builder, so the command line of the preview agrees with the job.
//!
//! A profile parameter expands as `{{param.<name>}}` in the paths, the excludes, the exclude files
//! and the tags, and it gives the value of a script variable (FR-PF-10, ADR-0047). A secret
//! parameter never expands: the restic arguments are visible in the process list (FR-AG-06).

pub mod arg;
mod compile;
mod error;
mod job;
mod option;
pub mod phase;
mod preview;
mod profile;
pub mod script;
mod target;
mod template;

pub use arg::{ArgError, CommandLine};
pub use compile::{Plan, PlannedStep, compile};
pub use error::{CompileError, CronError, FieldPath, validate_cron};
pub use job::ValidatedJob;
pub use option::ResticOption;
pub use preview::{PreviewError, PreviewStep, masked_command, preview};
pub use profile::{
    BackupOptions, CheckOptions, GroupBy, HookPhase, JobLimits, Profile, ProfilePaths, Retention,
    SnapshotFilter,
};
pub use script::{
    FailurePolicy, ScriptEnvRef, ScriptEnvValue, ScriptInterpreter, ScriptRef, ServerScripts,
    check_script,
};
pub use target::{Architecture, OperatingSystem, ParameterValue, ResticVersion, TargetFacts};
pub use template::{TemplateError, parameter_names};

/// Reports `incompatible` when the installed restic is older than the option
/// needs (AC-AG-09).
///
/// # Errors
///
/// Returns [`CompileError::IncompatibleOption`] with the option name and the
/// necessary version.
pub fn check_restic_version(
    option: ResticOption,
    installed: ResticVersion,
) -> Result<(), CompileError> {
    let required = option.minimum_version();
    if installed < required {
        return Err(CompileError::IncompatibleOption {
            field: FieldPath::new(option.flag()),
            option,
            required,
            installed,
        });
    }
    Ok(())
}
