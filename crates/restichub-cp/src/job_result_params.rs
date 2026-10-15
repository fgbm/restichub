//! The typed values behind the result message `incompatible` (AC-AG-09).
//!
//! The user interface and the alert text build their words from these values, so the stored
//! message is a stable code and not prose (NFR-DOC-04).

use restichub_compiler::CompileError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// The result message of a job that the restic of the agent cannot run.
pub const INCOMPATIBLE_CODE: &str = "incompatible";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct IncompatibleParams {
    /// The restic option that needs a newer version. Null when the whole restic version is too old.
    pub option: Option<String>,
    /// The version that the option needs, such as `0.19.0`.
    pub required_version: String,
    /// The restic version of the agent, such as `0.18.1`.
    pub installed_version: String,
}

impl IncompatibleParams {
    /// `None` when the error is not a version mismatch.
    pub fn of(error: &CompileError) -> Option<Self> {
        match error {
            CompileError::IncompatibleOption {
                option,
                required,
                installed,
                ..
            } => Some(Self {
                option: Some(option.flag().to_owned()),
                required_version: required.to_string(),
                installed_version: installed.to_string(),
            }),
            CompileError::UnsupportedRestic {
                installed,
                required,
            } => Some(Self {
                option: None,
                required_version: required.to_string(),
                installed_version: installed.to_string(),
            }),
            _ => None,
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// `None` for a value that does not hold the three fields.
    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

#[cfg(test)]
mod tests {
    use restichub_compiler::{FieldPath, ResticOption, ResticVersion};

    use super::*;

    #[test]
    fn ac_ag_09_the_values_name_the_option_and_the_versions() {
        let error = CompileError::IncompatibleOption {
            field: FieldPath::new("--ownership-by-name"),
            option: ResticOption::RestoreOwnershipByName,
            required: ResticVersion {
                major: 0,
                minor: 19,
                patch: 0,
            },
            installed: ResticVersion::MIN_SUPPORTED,
        };
        let params = IncompatibleParams::of(&error).expect("a version mismatch");
        assert_eq!(
            params,
            IncompatibleParams {
                option: Some("--ownership-by-name".to_owned()),
                required_version: "0.19.0".to_owned(),
                installed_version: "0.18.1".to_owned(),
            }
        );
        assert_eq!(
            IncompatibleParams::from_value(&params.to_value()),
            Some(params)
        );
    }

    #[test]
    fn another_compile_error_has_no_values() {
        let error = CompileError::NoPaths {
            os: restichub_compiler::OperatingSystem::Linux,
        };
        assert_eq!(IncompatibleParams::of(&error), None);
    }
}
