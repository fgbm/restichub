//! Compiler errors with the field that caused them (AC-PF-02).

use std::fmt;

use restichub_proto::agent::v1::RejectReason;

use crate::option::ResticOption;
use crate::target::{OperatingSystem, ResticVersion};

/// The path of a profile field, for example `paths.all[0]` (AC-PF-02).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPath(String);

impl FieldPath {
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A profile that the compiler rejects, with the field that caused it (FR-PF-05).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
    #[error("unknown template variable {name:?} in {field}")]
    UnknownVariable { field: FieldPath, name: String },
    #[error("agent label {key:?} is missing for {field}")]
    MissingLabel { field: FieldPath, key: String },
    #[error("invalid date format {format:?} in {field}")]
    InvalidDate { field: FieldPath, format: String },
    #[error("unterminated template in {field}")]
    UnterminatedTemplate { field: FieldPath },
    #[error("invalid value in {field}")]
    InvalidValue { field: FieldPath },
    #[error("invalid cron expression {expression:?}")]
    InvalidCron { expression: String },
    #[error("the profile has no path for {os}")]
    NoPaths { os: OperatingSystem },
    #[error("the host does not accept scripts from the control plane ({field})")]
    ServerScriptsDisabled { field: FieldPath },
    #[error("the host has no {interpreter:?} interpreter ({field})")]
    InterpreterUnavailable {
        field: FieldPath,
        interpreter: crate::script::ScriptInterpreter,
    },
    #[error("option {option} is not available on {os} ({field})")]
    UnsupportedOs {
        field: FieldPath,
        option: ResticOption,
        os: OperatingSystem,
    },
    #[error("option {option} needs restic {required}, the agent has {installed} ({field})")]
    IncompatibleOption {
        field: FieldPath,
        option: ResticOption,
        required: ResticVersion,
        installed: ResticVersion,
    },
    #[error("the agent has restic {installed}, the minimum is {required}")]
    UnsupportedRestic {
        installed: ResticVersion,
        required: ResticVersion,
    },
    #[error("the step contract rejects the plan: {reason:?}")]
    StepsRejected { reason: RejectReason },
}

/// A cron expression that the control plane rejects (FR-PF-05, AC-PF-02).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CronError {
    #[error("a cron expression has five fields")]
    FieldCount,
    #[error("invalid value {value:?} in the {field} field")]
    Value { field: &'static str, value: String },
}

const FIELDS: [(&str, u32, u32); 5] = [
    ("minute", 0, 59),
    ("hour", 0, 23),
    ("day of month", 1, 31),
    ("month", 1, 12),
    ("day of week", 0, 7),
];

/// Validates a five-field cron expression (FR-SC-01, AC-PF-02).
///
/// Each field accepts `*`, a number, a range `a-b`, a step `*/n` or `a-b/n`,
/// and a comma-separated list of these.
///
/// # Errors
///
/// Returns a [`CronError`] for a wrong field count or an out-of-range value.
pub fn validate_cron(expression: &str) -> Result<(), CronError> {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != FIELDS.len() {
        return Err(CronError::FieldCount);
    }
    for (value, (name, min, max)) in fields.iter().zip(FIELDS) {
        validate_cron_field(value, name, min, max)?;
    }
    Ok(())
}

fn validate_cron_field(
    value: &str,
    name: &'static str,
    min: u32,
    max: u32,
) -> Result<(), CronError> {
    for item in value.split(',') {
        let invalid = || CronError::Value {
            field: name,
            value: item.to_owned(),
        };
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => (range, Some(step)),
            None => (item, None),
        };
        if let Some(step) = step {
            let step: u32 = step.parse().map_err(|_| invalid())?;
            if step == 0 {
                return Err(invalid());
            }
        }
        if range == "*" {
            continue;
        }
        match range.split_once('-') {
            Some((start, end)) => {
                let start: u32 = start.parse().map_err(|_| invalid())?;
                let end: u32 = end.parse().map_err(|_| invalid())?;
                if start < min || end > max || start > end {
                    return Err(invalid());
                }
            }
            None => {
                let number: u32 = range.parse().map_err(|_| invalid())?;
                if number < min || number > max {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}

impl CompileError {
    /// True when the error comes from the scripts of the profile: the host does not accept them
    /// or lacks the interpreter (FR-HK-10).
    #[must_use]
    pub fn is_script(&self) -> bool {
        matches!(
            self,
            Self::ServerScriptsDisabled { .. } | Self::InterpreterUnavailable { .. }
        )
    }

    #[must_use]
    pub fn field(&self) -> Option<&FieldPath> {
        match self {
            Self::UnknownVariable { field, .. }
            | Self::MissingLabel { field, .. }
            | Self::InvalidDate { field, .. }
            | Self::UnterminatedTemplate { field }
            | Self::InvalidValue { field }
            | Self::ServerScriptsDisabled { field }
            | Self::InterpreterUnavailable { field, .. }
            | Self::UnsupportedOs { field, .. }
            | Self::IncompatibleOption { field, .. } => Some(field),
            Self::InvalidCron { .. }
            | Self::NoPaths { .. }
            | Self::UnsupportedRestic { .. }
            | Self::StepsRejected { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_cron;

    #[test]
    fn ac_pf_02_incorrect_cron_is_rejected() {
        let cases = [
            "",
            "* * * *",
            "* * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * 32 * *",
            "* * * 13 *",
            "* * * * 8",
            "a * * * *",
            "*/0 * * * *",
            "* * 5-1 * *",
            "* * * * 1-8",
        ];
        for case in cases {
            assert!(validate_cron(case).is_err(), "{case}");
        }
    }

    #[test]
    fn valid_cron_is_accepted() {
        let cases = [
            "* * * * *",
            "0 3 * * *",
            "*/15 0-23 1,15 * 1-5",
            "0 0 1 1 0",
            "0 6,18 * * 0,7",
        ];
        for case in cases {
            assert!(validate_cron(case).is_ok(), "{case}");
        }
    }
}
