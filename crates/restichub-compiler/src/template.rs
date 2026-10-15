//! Template variables (FR-PF-03).
//!
//! A variable is `{{name}}`. The supported names are `hostname`, `os`,
//! `agent.name`, `agent.label.<key>`, `date` and `date:<format>`. A date format
//! uses the directives `%Y`, `%m`, `%d`, `%H`, `%M`, `%S` and `%%`. Expansion
//! runs once; a substituted value is not expanded again.

use time::OffsetDateTime;

use crate::target::TargetFacts;

/// A template that the compiler cannot expand (FR-PF-03).
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
}

/// Substitutes the variables in `value` (FR-PF-03).
///
/// # Errors
///
/// Returns a [`TemplateError`] for an unknown variable, a missing label, an
/// invalid date format or a missing `}}`.
pub fn expand(value: &str, target: &TargetFacts) -> Result<String, TemplateError> {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(TemplateError::Unterminated);
        };
        output.push_str(&resolve(after[..end].trim(), target)?);
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

fn resolve(expression: &str, target: &TargetFacts) -> Result<String, TemplateError> {
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

    use super::{TemplateError, expand};
    use crate::target::{Architecture, OperatingSystem, ResticVersion, TargetFacts};

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
            restic_version: ResticVersion::MIN_SUPPORTED,
            now: date(2026, 10, 13),
            allow_list: BTreeMap::new(),
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
