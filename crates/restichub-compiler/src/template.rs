//! Template variables (FR-PF-03, FR-PF-10).
//!
//! A variable is `{{name}}`. The supported names are `hostname`, `os`, `agent.name`,
//! `agent.label.<key>`, `date`, `date:<format>` and `param.<name>`. A date format uses the
//! directives `%Y`, `%m`, `%d`, `%H`, `%M`, `%S` and `%%`. Expansion runs once; a substituted value
//! is not expanded again.
//!
//! A parameter expands in the paths, the excludes, the exclude files and the tags only. The
//! retention and check fields pass [`expand_without_parameters`], because one maintenance step
//! serves every host (ADR-0034, ADR-0047). A secret parameter never expands: the restic arguments
//! are visible in the process list (FR-AG-06).

use time::OffsetDateTime;

use crate::target::{ParameterValue, TargetFacts};

/// A template that the compiler cannot expand (FR-PF-03, FR-PF-10).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("unknown variable {name:?}")]
    UnknownVariable { name: String },
    #[error("agent label {key:?} is missing")]
    MissingLabel { key: String },
    #[error("invalid date format {format:?}")]
    InvalidDate { format: String },
    #[error("unterminated variable")]
    Unterminated,
    #[error("the parameter {name:?} has no value")]
    MissingParameter { name: String },
    #[error("the parameter {name:?} is a secret and cannot be expanded")]
    SecretParameter { name: String },
    #[error("the parameter {name:?} cannot expand in this field")]
    ParameterNotAllowed { name: String },
}

/// Substitutes the variables in `value` (FR-PF-03).
///
/// # Errors
///
/// Returns a [`TemplateError`] for an unknown variable, a missing label, an invalid date format, a
/// missing parameter or a missing `}}`.
pub fn expand(value: &str, target: &TargetFacts) -> Result<String, TemplateError> {
    expand_with(value, target, true)
}

/// Substitutes the variables in `value` without a parameter (FR-PF-10, ADR-0047).
///
/// # Errors
///
/// Returns [`TemplateError::ParameterNotAllowed`] for a parameter variable and the same errors as
/// [`expand`].
pub fn expand_without_parameters(
    value: &str,
    target: &TargetFacts,
) -> Result<String, TemplateError> {
    expand_with(value, target, false)
}

/// The names of the parameters that `value` names, in the order of the value and without a repeat.
///
/// The save-time check reads it to refuse a template that a value can never give (FR-PF-05).
#[must_use]
pub fn parameter_names(value: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        if let Some(name) = after[..end].trim().strip_prefix("param.")
            && !names.contains(&name)
        {
            names.push(name);
        }
        rest = &after[end + 2..];
    }
    names
}

fn expand_with(
    value: &str,
    target: &TargetFacts,
    allow_parameters: bool,
) -> Result<String, TemplateError> {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(TemplateError::Unterminated);
        };
        output.push_str(&resolve(after[..end].trim(), target, allow_parameters)?);
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

fn resolve(
    expression: &str,
    target: &TargetFacts,
    allow_parameters: bool,
) -> Result<String, TemplateError> {
    if let Some(name) = expression.strip_prefix("param.") {
        if !allow_parameters {
            return Err(TemplateError::ParameterNotAllowed {
                name: name.to_owned(),
            });
        }
        return match target.parameters.get(name) {
            Some(ParameterValue::Text(value)) => Ok(value.clone()),
            Some(ParameterValue::Secret(_)) => Err(TemplateError::SecretParameter {
                name: name.to_owned(),
            }),
            None => Err(TemplateError::MissingParameter {
                name: name.to_owned(),
            }),
        };
    }
    match expression {
        "hostname" => Ok(target.hostname.clone()),
        "os" => Ok(target.os.as_str().to_owned()),
        "agent.name" => Ok(target.agent_name.clone()),
        "date" => format_date(target.now, "%Y-%m-%d"),
        _ => {
            if let Some(key) = expression.strip_prefix("agent.label.") {
                return target.labels.get(key).cloned().ok_or_else(|| {
                    TemplateError::MissingLabel {
                        key: key.to_owned(),
                    }
                });
            }
            if let Some(format) = expression.strip_prefix("date:") {
                return format_date(target.now, format);
            }
            Err(TemplateError::UnknownVariable {
                name: expression.to_owned(),
            })
        }
    }
}

fn format_date(now: OffsetDateTime, format: &str) -> Result<String, TemplateError> {
    let mut output = String::with_capacity(format.len());
    let mut chars = format.chars();
    while let Some(character) = chars.next() {
        if character != '%' {
            output.push(character);
            continue;
        }
        let Some(directive) = chars.next() else {
            return Err(TemplateError::InvalidDate {
                format: format.to_owned(),
            });
        };
        match directive {
            'Y' => output.push_str(&format!("{:04}", now.year())),
            'm' => output.push_str(&format!("{:02}", u8::from(now.month()))),
            'd' => output.push_str(&format!("{:02}", now.day())),
            'H' => output.push_str(&format!("{:02}", now.hour())),
            'M' => output.push_str(&format!("{:02}", now.minute())),
            'S' => output.push_str(&format!("{:02}", now.second())),
            '%' => output.push('%'),
            _ => {
                return Err(TemplateError::InvalidDate {
                    format: format.to_owned(),
                });
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use time::{Date, Month, OffsetDateTime};

    use super::{TemplateError, expand, expand_without_parameters, parameter_names};
    use crate::target::{
        Architecture, OperatingSystem, ParameterValue, ResticVersion, TargetFacts,
    };

    fn date(year: i32, month: u8, day: u8) -> OffsetDateTime {
        let month = Month::try_from(month).unwrap_or(Month::January);
        match Date::from_calendar_date(year, month, day) {
            Ok(value) => value.midnight().assume_utc(),
            Err(_) => panic!("invalid test date"),
        }
    }

    fn target() -> TargetFacts {
        let mut labels = BTreeMap::new();
        labels.insert("env".to_owned(), "prod".to_owned());
        TargetFacts {
            agent_id: "agent-1".to_owned(),
            agent_name: "web-1".to_owned(),
            hostname: "host.example.com".to_owned(),
            os: OperatingSystem::Linux,
            arch: Architecture::X86_64,
            labels,
            parameters: BTreeMap::new(),
            restic_version: ResticVersion::MIN_SUPPORTED,
            now: date(2026, 10, 13),
            server_scripts: crate::script::ServerScripts::default(),
        }
    }

    fn value(template: &str) -> Option<String> {
        expand(template, &target()).ok()
    }

    #[test]
    fn ac_pf_03_expands_hostname_os_label_and_date() {
        assert_eq!(value("{{hostname}}").as_deref(), Some("host.example.com"));
        assert_eq!(value("{{os}}").as_deref(), Some("linux"));
        assert_eq!(value("{{agent.name}}").as_deref(), Some("web-1"));
        assert_eq!(value("{{agent.label.env}}").as_deref(), Some("prod"));
        assert_eq!(value("{{date}}").as_deref(), Some("2026-10-13"));
        assert_eq!(
            value("backup-{{date:%Y%m%d}}").as_deref(),
            Some("backup-20261013")
        );
        assert_eq!(
            value("/srv/{{hostname}}/{{agent.label.env}}").as_deref(),
            Some("/srv/host.example.com/prod")
        );
    }

    #[test]
    fn ac_pf_08_a_text_parameter_expands() {
        let mut target = target();
        target.parameters.insert(
            "pg_dir".to_owned(),
            ParameterValue::Text("/srv/pg".to_owned()),
        );
        assert_eq!(
            expand("/data/{{param.pg_dir}}", &target).ok().as_deref(),
            Some("/data//srv/pg")
        );
    }

    #[test]
    fn ac_pf_09_a_secret_parameter_does_not_expand() {
        let mut target = target();
        target.parameters.insert(
            "pg_password".to_owned(),
            ParameterValue::Secret("0192a1e0-0000-7000-8000-000000000001".to_owned()),
        );
        assert_eq!(
            expand("{{param.pg_password}}", &target),
            Err(TemplateError::SecretParameter {
                name: "pg_password".to_owned()
            })
        );
    }

    #[test]
    fn a_parameter_without_a_value_is_missing() {
        assert_eq!(
            expand("{{param.pg_dir}}", &target()),
            Err(TemplateError::MissingParameter {
                name: "pg_dir".to_owned()
            })
        );
    }

    #[test]
    fn a_parameter_does_not_expand_without_parameters() {
        let mut target = target();
        target.parameters.insert(
            "pg_dir".to_owned(),
            ParameterValue::Text("/srv/pg".to_owned()),
        );
        assert_eq!(
            expand_without_parameters("{{param.pg_dir}}", &target),
            Err(TemplateError::ParameterNotAllowed {
                name: "pg_dir".to_owned()
            })
        );
        assert_eq!(
            expand_without_parameters("{{hostname}}", &target)
                .ok()
                .as_deref(),
            Some("host.example.com")
        );
    }

    #[test]
    fn parameter_names_lists_each_parameter_once() {
        assert_eq!(
            parameter_names("/srv/{{param.a}}/{{ param.b }}/{{param.a}}/{{hostname}}"),
            ["a", "b"]
        );
        assert!(parameter_names("/srv/{{hostname}}").is_empty());
    }

    #[test]
    fn unknown_variable_is_rejected() {
        assert!(matches!(
            expand("{{nope}}", &target()),
            Err(TemplateError::UnknownVariable { .. })
        ));
    }

    #[test]
    fn missing_label_is_rejected() {
        assert!(matches!(
            expand("{{agent.label.missing}}", &target()),
            Err(TemplateError::MissingLabel { .. })
        ));
    }

    #[test]
    fn invalid_date_format_is_rejected() {
        assert!(matches!(
            expand("{{date:%Q}}", &target()),
            Err(TemplateError::InvalidDate { .. })
        ));
    }

    #[test]
    fn unterminated_variable_is_rejected() {
        assert!(matches!(
            expand("{{hostname", &target()),
            Err(TemplateError::Unterminated)
        ));
    }

    #[test]
    fn substituted_value_is_not_expanded_again() {
        let mut target = target();
        target.hostname = "{{os}}".to_owned();
        assert_eq!(
            expand("{{hostname}}", &target).ok().as_deref(),
            Some("{{os}}")
        );
    }
}
