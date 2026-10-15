//! The typed schema of an allow-list hook and the check of its parameter values (FR-HK-02,
//! FR-HK-03, ADR-0004).
//!
//! The agent loads the schema from its allow-list file and checks every value of a job against it.
//! It reports the public part of the schema to the control plane, which checks the values of a
//! profile against the same rules when it saves the profile and when it compiles a job. Both sides
//! use this module, so the rules cannot drift.

use std::collections::BTreeMap;

use regex::RegexBuilder;
use restichub_proto::agent::v1::{
    HookMetadata, HookParameterSpec as ProtoSpec, HookParameterType, HookPhase as ProtoPhase,
    hook_parameter::Typed,
};
use serde::{Deserialize, Serialize};

use crate::profile::HookPhase;

/// The longest string value when the schema sets no limit (FR-HK-02).
pub const DEFAULT_MAX_LENGTH: u32 = 256;

/// The largest length that a schema may set (FR-HK-02).
pub const MAX_STRING_LENGTH: u32 = 4096;

/// Upper bound of the parameters, so reported metadata cannot grow without limit (FR-HK-02).
pub const MAX_PARAMETERS: usize = 32;

/// Upper bound of the values in one enumeration.
pub const MAX_ENUM_VALUES: usize = 64;

/// The longest identifier of a hook or a parameter.
pub const MAX_NAME: usize = 64;

/// The largest compiled size of a parameter pattern, so a pattern cannot use much memory.
const PATTERN_SIZE_LIMIT: usize = 1 << 20;

/// True for the identifier of a hook: 1 to [`MAX_NAME`] characters from `a-z`, `0-9`, `.`, `_` and
/// `-`, and a letter or a digit first.
#[must_use]
pub fn is_valid_hook_id(value: &str) -> bool {
    is_valid_name(value, |character| matches!(character, '.' | '_' | '-'))
}

/// True for the name of a hook parameter: 1 to [`MAX_NAME`] characters from `a-z`, `0-9`, `_` and
/// `-`, and a letter or a digit first. The name is part of a `--name=value` argument.
#[must_use]
pub fn is_valid_parameter_name(value: &str) -> bool {
    is_valid_name(value, |character| matches!(character, '_' | '-'))
}

fn is_valid_name(value: &str, extra: fn(char) -> bool) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    value.len() <= MAX_NAME
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || extra(character)
        })
}

/// The type of a hook parameter with the limits of the schema (FR-HK-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterKind {
    /// A string of at most `max_length` bytes. When `pattern` is set, the whole string must match.
    String {
        max_length: u32,
        pattern: Option<String>,
    },
    /// A whole number between `minimum` and `maximum` when they are set.
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    /// One value of a closed list.
    Enum {
        values: Vec<String>,
    },
    Boolean,
    /// A reference to a secret of the control plane (ADR-0017).
    SecretRef,
}

/// One parameter of a hook (FR-HK-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookParameterSpec {
    pub kind: ParameterKind,
    /// True when every use of the hook must set the parameter.
    pub required: bool,
}

impl HookParameterSpec {
    /// True when the value is a secret, so the preview masks it (AC-PF-01).
    #[must_use]
    pub fn secret(&self) -> bool {
        matches!(self.kind, ParameterKind::SecretRef)
    }
}

/// One hook of the agent allow-list that the agent reported (FR-HK-04). It holds no path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowListedHook {
    /// Accepted arguments, by name (FR-HK-02).
    pub parameters: BTreeMap<String, HookParameterSpec>,
    /// Where the hook may run. An empty list allows every phase.
    pub phases: Vec<HookPhase>,
}

impl AllowListedHook {
    /// True when the hook may run in the phase.
    #[must_use]
    pub fn allows(&self, phase: HookPhase) -> bool {
        self.phases.is_empty() || self.phases.contains(&phase)
    }
}

/// A parameter value of the closed set of types (FR-HK-03).
#[derive(Clone, PartialEq, Eq)]
pub enum HookValue {
    String(String),
    Integer(i64),
    Boolean(bool),
    /// A secret reference. At the agent it holds the resolved secret.
    Secret(String),
}

impl std::fmt::Debug for HookValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(value) => formatter.debug_tuple("String").field(value).finish(),
            Self::Integer(value) => formatter.debug_tuple("Integer").field(value).finish(),
            Self::Boolean(value) => formatter.debug_tuple("Boolean").field(value).finish(),
            Self::Secret(_) => formatter.write_str("Secret(***)"),
        }
    }
}

impl HookValue {
    /// Reads the oneof of a protocol parameter.
    #[must_use]
    pub fn from_proto(typed: &Typed) -> Self {
        match typed {
            Typed::StringValue(value) => Self::String(value.clone()),
            Typed::IntegerValue(value) => Self::Integer(*value),
            Typed::BooleanValue(value) => Self::Boolean(*value),
            Typed::SecretValue(value) => Self::Secret(value.clone()),
        }
    }

    #[must_use]
    pub fn to_proto(&self) -> Typed {
        match self {
            Self::String(value) => Typed::StringValue(value.clone()),
            Self::Integer(value) => Typed::IntegerValue(*value),
            Self::Boolean(value) => Typed::BooleanValue(*value),
            Self::Secret(value) => Typed::SecretValue(value.clone()),
        }
    }
}

/// Why a value does not obey the schema of its parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("the value has another type than the parameter")]
    WrongType,
    #[error("the value holds a control character")]
    ControlCharacter,
    #[error("the value is longer than the limit")]
    TooLong,
    #[error("the value does not match the pattern")]
    PatternMismatch,
    #[error("the pattern of the schema is not valid")]
    BadPattern,
    #[error("the integer is outside the limits")]
    OutOfRange,
    #[error("the value is not in the list")]
    NotInList,
    #[error("the value is empty")]
    Empty,
}

impl HookParameterSpec {
    /// Compares a typed value with the schema of the parameter.
    ///
    /// # Errors
    ///
    /// Returns the first rule that the value breaks, as a [`ValueError`].
    pub fn check(&self, value: &HookValue) -> Result<(), ValueError> {
        match (&self.kind, value) {
            (
                ParameterKind::String {
                    max_length,
                    pattern,
                },
                HookValue::String(text),
            ) => {
                check_text(text)?;
                let limit = usize::try_from(*max_length).unwrap_or(usize::MAX);
                if text.len() > limit {
                    return Err(ValueError::TooLong);
                }
                match pattern {
                    Some(pattern) => check_pattern(pattern, text),
                    None => Ok(()),
                }
            }
            (ParameterKind::Integer { minimum, maximum }, HookValue::Integer(number)) => {
                if minimum.is_some_and(|minimum| *number < minimum)
                    || maximum.is_some_and(|maximum| *number > maximum)
                {
                    Err(ValueError::OutOfRange)
                } else {
                    Ok(())
                }
            }
            (ParameterKind::Enum { values }, HookValue::String(text)) => {
                if values.contains(text) {
                    Ok(())
                } else {
                    Err(ValueError::NotInList)
                }
            }
            (ParameterKind::Boolean, HookValue::Boolean(_)) => Ok(()),
            (ParameterKind::SecretRef, HookValue::Secret(text)) => {
                // A secret may hold line breaks, such as a key. Only NUL cannot travel in an
                // environment variable.
                if text.contains('\0') {
                    return Err(ValueError::ControlCharacter);
                }
                if text.is_empty() {
                    Err(ValueError::Empty)
                } else {
                    Ok(())
                }
            }
            _ => Err(ValueError::WrongType),
        }
    }

    /// Reads the text form of a profile value and checks it. An integer is decimal text, a boolean
    /// is `true` or `false`, and a secret reference is the identifier of the secret.
    ///
    /// # Errors
    ///
    /// Returns the first rule that the value breaks, as a [`ValueError`].
    pub fn parse_text(&self, text: &str) -> Result<HookValue, ValueError> {
        let value = match &self.kind {
            ParameterKind::String { .. } | ParameterKind::Enum { .. } => {
                HookValue::String(text.to_owned())
            }
            ParameterKind::Integer { .. } => {
                HookValue::Integer(text.parse().map_err(|_| ValueError::WrongType)?)
            }
            ParameterKind::Boolean => match text {
                "true" => HookValue::Boolean(true),
                "false" => HookValue::Boolean(false),
                _ => return Err(ValueError::WrongType),
            },
            ParameterKind::SecretRef => HookValue::Secret(text.to_owned()),
        };
        self.check(&value)?;
        Ok(value)
    }
}

/// A control, format or separator character never belongs to a hook value (FR-PR-10).
fn check_text(text: &str) -> Result<(), ValueError> {
    if restichub_proto::validate::has_forbidden(text) {
        Err(ValueError::ControlCharacter)
    } else {
        Ok(())
    }
}

/// Compiles a pattern that must match the whole value. The regex engine runs in linear time, so a
/// value cannot make it hang.
///
/// # Errors
///
/// Returns [`ValueError::BadPattern`] when the pattern does not compile within the size limit.
pub fn compile_pattern(pattern: &str) -> Result<regex::Regex, ValueError> {
    RegexBuilder::new(&format!("^(?:{pattern})$"))
        .size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .map_err(|_| ValueError::BadPattern)
}

fn check_pattern(pattern: &str, text: &str) -> Result<(), ValueError> {
    if compile_pattern(pattern)?.is_match(text) {
        Ok(())
    } else {
        Err(ValueError::PatternMismatch)
    }
}

/// Why the control plane cannot use the metadata of a hook that an agent reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MetadataError {
    #[error("the hook identifier is not valid")]
    HookId,
    #[error("a parameter name is not valid or repeats")]
    ParameterName,
    #[error("a parameter has a type that this version does not know")]
    ParameterType,
    #[error("the metadata is larger than the limits")]
    TooLarge,
}

impl AllowListedHook {
    /// Reads the metadata that an agent reported in its `AgentHello` (FR-HK-04).
    ///
    /// The agent is trusted for its own allow-list, but the control plane still bounds the data,
    /// so a broken agent cannot fill the database.
    ///
    /// # Errors
    ///
    /// Returns the rule that the metadata breaks, as a [`MetadataError`].
    pub fn from_proto(metadata: &HookMetadata) -> Result<(String, Self), MetadataError> {
        if !is_valid_hook_id(&metadata.hook_id) {
            return Err(MetadataError::HookId);
        }
        if metadata.parameters.len() > MAX_PARAMETERS || metadata.phases.len() > 5 {
            return Err(MetadataError::TooLarge);
        }
        let mut parameters = BTreeMap::new();
        for parameter in &metadata.parameters {
            if !is_valid_parameter_name(&parameter.name) {
                return Err(MetadataError::ParameterName);
            }
            let spec = HookParameterSpec {
                kind: kind_from_proto(parameter)?,
                required: parameter.required,
            };
            if parameters.insert(parameter.name.clone(), spec).is_some() {
                return Err(MetadataError::ParameterName);
            }
        }
        let mut phases = Vec::new();
        for phase in &metadata.phases {
            let phase = phase_from_proto(*phase).ok_or(MetadataError::TooLarge)?;
            if !phases.contains(&phase) {
                phases.push(phase);
            }
        }
        Ok((metadata.hook_id.clone(), Self { parameters, phases }))
    }

    /// The public metadata for the `AgentHello` of the agent.
    #[must_use]
    pub fn to_proto(&self, hook_id: &str) -> HookMetadata {
        HookMetadata {
            hook_id: hook_id.to_owned(),
            parameters: self
                .parameters
                .iter()
                .map(|(name, spec)| spec_to_proto(name, spec))
                .collect(),
            phases: self
                .phases
                .iter()
                .map(|phase| phase_to_proto(*phase) as i32)
                .collect(),
        }
    }
}

fn kind_from_proto(parameter: &ProtoSpec) -> Result<ParameterKind, MetadataError> {
    let kind =
        HookParameterType::try_from(parameter.r#type).map_err(|_| MetadataError::ParameterType)?;
    Ok(match kind {
        HookParameterType::String => {
            if parameter.max_length > MAX_STRING_LENGTH || parameter.pattern.len() > 1024 {
                return Err(MetadataError::TooLarge);
            }
            ParameterKind::String {
                max_length: if parameter.max_length == 0 {
                    DEFAULT_MAX_LENGTH
                } else {
                    parameter.max_length
                },
                pattern: (!parameter.pattern.is_empty()).then(|| parameter.pattern.clone()),
            }
        }
        HookParameterType::Integer => ParameterKind::Integer {
            minimum: parameter.minimum,
            maximum: parameter.maximum,
        },
        HookParameterType::Enum => {
            if parameter.enum_values.len() > MAX_ENUM_VALUES
                || parameter
                    .enum_values
                    .iter()
                    .any(|value| value.len() > MAX_NAME)
            {
                return Err(MetadataError::TooLarge);
            }
            ParameterKind::Enum {
                values: parameter.enum_values.clone(),
            }
        }
        HookParameterType::Boolean => ParameterKind::Boolean,
        HookParameterType::SecretRef => ParameterKind::SecretRef,
        HookParameterType::Unspecified => return Err(MetadataError::ParameterType),
    })
}

fn spec_to_proto(name: &str, spec: &HookParameterSpec) -> ProtoSpec {
    let mut proto = ProtoSpec {
        name: name.to_owned(),
        required: spec.required,
        ..ProtoSpec::default()
    };
    match &spec.kind {
        ParameterKind::String {
            max_length,
            pattern,
        } => {
            proto.r#type = HookParameterType::String as i32;
            proto.max_length = *max_length;
            proto.pattern = pattern.clone().unwrap_or_default();
        }
        ParameterKind::Integer { minimum, maximum } => {
            proto.r#type = HookParameterType::Integer as i32;
            proto.minimum = *minimum;
            proto.maximum = *maximum;
        }
        ParameterKind::Enum { values } => {
            proto.r#type = HookParameterType::Enum as i32;
            proto.enum_values.clone_from(values);
        }
        ParameterKind::Boolean => proto.r#type = HookParameterType::Boolean as i32,
        ParameterKind::SecretRef => proto.r#type = HookParameterType::SecretRef as i32,
    }
    proto
}

fn phase_from_proto(value: i32) -> Option<HookPhase> {
    match ProtoPhase::try_from(value).ok()? {
        ProtoPhase::Before => Some(HookPhase::Before),
        ProtoPhase::After => Some(HookPhase::After),
        ProtoPhase::OnSuccess => Some(HookPhase::OnSuccess),
        ProtoPhase::OnError => Some(HookPhase::OnError),
        ProtoPhase::Finally => Some(HookPhase::Finally),
        ProtoPhase::Unspecified => None,
    }
}

/// Maps a phase to its protocol value.
#[must_use]
pub fn phase_to_proto(phase: HookPhase) -> ProtoPhase {
    match phase {
        HookPhase::Before => ProtoPhase::Before,
        HookPhase::After => ProtoPhase::After,
        HookPhase::OnSuccess => ProtoPhase::OnSuccess,
        HookPhase::OnError => ProtoPhase::OnError,
        HookPhase::Finally => ProtoPhase::Finally,
    }
}

/// The phase of a protocol value, or `None` for an unspecified or unknown value.
#[must_use]
pub fn phase_of(value: i32) -> Option<HookPhase> {
    phase_from_proto(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(pattern: Option<&str>, max_length: u32) -> HookParameterSpec {
        HookParameterSpec {
            kind: ParameterKind::String {
                max_length,
                pattern: pattern.map(str::to_owned),
            },
            required: true,
        }
    }

    #[test]
    fn fr_hk_02_a_pattern_must_match_the_whole_value() {
        let spec = string(Some("[a-z]+"), 16);
        assert!(spec.check(&HookValue::String("abc".into())).is_ok());
        for bad in ["abc; rm -rf /", "$(id)", "`id`", "ab c", "", "ABC", "abc\n"] {
            assert!(
                spec.check(&HookValue::String(bad.into())).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn fr_hk_02_an_alternation_cannot_escape_the_anchors() {
        let spec = string(Some("a|b"), 16);
        assert!(spec.check(&HookValue::String("a".into())).is_ok());
        assert_eq!(
            spec.check(&HookValue::String("a; rm -rf /".into())),
            Err(ValueError::PatternMismatch)
        );
        assert_eq!(
            spec.check(&HookValue::String("xb".into())),
            Err(ValueError::PatternMismatch)
        );
    }

    #[test]
    fn fr_hk_02_a_string_obeys_its_length_and_has_no_control_character() {
        let spec = string(None, 4);
        assert!(spec.check(&HookValue::String("abcd".into())).is_ok());
        assert_eq!(
            spec.check(&HookValue::String("abcde".into())),
            Err(ValueError::TooLong)
        );
        assert_eq!(
            spec.check(&HookValue::String("a\0b".into())),
            Err(ValueError::ControlCharacter)
        );
    }

    #[test]
    fn fr_hk_02_an_integer_obeys_its_range() {
        let spec = HookParameterSpec {
            kind: ParameterKind::Integer {
                minimum: Some(1),
                maximum: Some(10),
            },
            required: false,
        };
        assert!(spec.check(&HookValue::Integer(10)).is_ok());
        assert_eq!(
            spec.check(&HookValue::Integer(0)),
            Err(ValueError::OutOfRange)
        );
        assert_eq!(
            spec.check(&HookValue::Integer(11)),
            Err(ValueError::OutOfRange)
        );
        assert_eq!(
            spec.check(&HookValue::String("5".into())),
            Err(ValueError::WrongType)
        );
        assert!(spec.parse_text("7").is_ok());
        assert_eq!(spec.parse_text("7; id"), Err(ValueError::WrongType));
    }

    #[test]
    fn fr_hk_02_an_enumeration_accepts_its_values_only() {
        let spec = HookParameterSpec {
            kind: ParameterKind::Enum {
                values: vec!["full".into(), "incremental".into()],
            },
            required: true,
        };
        assert!(spec.parse_text("full").is_ok());
        assert_eq!(spec.parse_text("FULL"), Err(ValueError::NotInList));
        assert_eq!(spec.parse_text("full;id"), Err(ValueError::NotInList));
    }

    #[test]
    fn fr_hk_02_a_boolean_reads_true_and_false_only() {
        let spec = HookParameterSpec {
            kind: ParameterKind::Boolean,
            required: false,
        };
        assert_eq!(spec.parse_text("true"), Ok(HookValue::Boolean(true)));
        assert_eq!(spec.parse_text("false"), Ok(HookValue::Boolean(false)));
        assert_eq!(spec.parse_text("yes"), Err(ValueError::WrongType));
    }

    #[test]
    fn fr_hk_02_a_secret_value_is_masked_in_debug_output() {
        let value = HookValue::Secret("CANARY-do-not-log".into());
        assert!(!format!("{value:?}").contains("CANARY"));
    }

    #[test]
    fn fr_hk_04_the_metadata_round_trips_through_the_protocol() {
        let hook = AllowListedHook {
            parameters: BTreeMap::from([
                ("database".to_owned(), string(Some("[a-z]+"), 32)),
                (
                    "mode".to_owned(),
                    HookParameterSpec {
                        kind: ParameterKind::Enum {
                            values: vec!["full".into()],
                        },
                        required: false,
                    },
                ),
                (
                    "token".to_owned(),
                    HookParameterSpec {
                        kind: ParameterKind::SecretRef,
                        required: true,
                    },
                ),
            ]),
            phases: vec![HookPhase::Before, HookPhase::Finally],
        };
        let metadata = hook.to_proto("pg-dump");
        assert_eq!(
            AllowListedHook::from_proto(&metadata),
            Ok(("pg-dump".to_owned(), hook))
        );
    }

    #[test]
    fn fr_hk_04_the_control_plane_refuses_broken_metadata() {
        let mut metadata = AllowListedHook::default().to_proto("pg-dump");
        metadata.hook_id = "Bad Id".into();
        assert_eq!(
            AllowListedHook::from_proto(&metadata),
            Err(MetadataError::HookId)
        );
        let mut metadata = AllowListedHook::default().to_proto("pg-dump");
        metadata.parameters.push(ProtoSpec {
            name: "x".into(),
            ..ProtoSpec::default()
        });
        assert_eq!(
            AllowListedHook::from_proto(&metadata),
            Err(MetadataError::ParameterType)
        );
        let mut metadata = AllowListedHook::default().to_proto("pg-dump");
        for _ in 0..2 {
            metadata.parameters.push(ProtoSpec {
                name: "x".into(),
                r#type: HookParameterType::Boolean as i32,
                ..ProtoSpec::default()
            });
        }
        assert_eq!(
            AllowListedHook::from_proto(&metadata),
            Err(MetadataError::ParameterName)
        );
    }

    #[test]
    fn fr_hk_02_names_are_plain() {
        assert!(is_valid_hook_id("pg-dump.v2"));
        assert!(!is_valid_hook_id(""));
        assert!(!is_valid_hook_id("-x"));
        assert!(!is_valid_hook_id("a b"));
        assert!(is_valid_parameter_name("db_name"));
        assert!(!is_valid_parameter_name("db.name"));
        assert!(!is_valid_parameter_name("--db"));
    }
}
