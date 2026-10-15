//! The allow-list file format (FR-HK-02, ADR-0004).
//!
//! The file is TOML. A host administrator writes it by hand or with a configuration management
//! tool. Every key is checked and an unknown key is an error, so a typo never loosens a rule.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use restichub_compiler::HookParameterSpec;
use restichub_compiler::HookPhase;
use restichub_compiler::ParameterKind;
use restichub_compiler::hook::{
    DEFAULT_MAX_LENGTH, MAX_ENUM_VALUES, MAX_NAME, MAX_PARAMETERS, MAX_STRING_LENGTH,
    compile_pattern, is_valid_hook_id, is_valid_parameter_name,
};
use restichub_proto::validate::has_forbidden;
use serde::Deserialize;

use super::{Hook, Mapping, Parameter};

/// The largest allow-list file that the agent reads.
pub(super) const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Upper bound of the hook entries, so a broken file cannot grow without limit.
const MAX_HOOKS: usize = 64;

/// The timeout of a hook that sets none (FR-HK-02).
const DEFAULT_TIMEOUT_SECONDS: u64 = 300;

/// Upper bound of `timeout_seconds`: one day.
const MAX_TIMEOUT_SECONDS: u64 = 24 * 3600;

/// Why the file is not valid, in words for the host administrator.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct FormatError(String);

fn fail<T>(message: impl Into<String>) -> Result<T, FormatError> {
    Err(FormatError(message.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(default)]
    hook: Vec<HookDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookDocument {
    id: String,
    executable: String,
    #[serde(default)]
    args: Vec<String>,
    timeout_seconds: Option<u64>,
    working_dir: Option<String>,
    phases: Option<Vec<PhaseDocument>>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    parameter: Vec<ParameterDocument>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum PhaseDocument {
    Before,
    After,
    OnSuccess,
    OnError,
    Finally,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TypeDocument {
    String,
    Integer,
    Enum,
    Boolean,
    SecretRef,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PassAs {
    Flag,
    Positional,
    Env,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParameterDocument {
    name: String,
    #[serde(rename = "type")]
    kind: TypeDocument,
    required: Option<bool>,
    max_length: Option<u32>,
    pattern: Option<String>,
    minimum: Option<i64>,
    maximum: Option<i64>,
    values: Option<Vec<String>>,
    pass_as: Option<PassAs>,
    position: Option<u32>,
    env_name: Option<String>,
}

/// Parses and validates the text of the allow-list file.
///
/// # Errors
///
/// Returns the first rule that the file breaks, with the identifier of the hook.
pub(super) fn parse(text: &str) -> Result<BTreeMap<String, Hook>, FormatError> {
    let document: Document =
        toml::from_str(text).map_err(|error| FormatError(error.message().to_owned()))?;
    if document.hook.len() > MAX_HOOKS {
        return fail(format!("the file holds more than {MAX_HOOKS} hooks"));
    }
    let mut hooks = BTreeMap::new();
    for hook in document.hook {
        let id = hook.id.clone();
        let parsed = convert(hook)
            .map_err(|FormatError(message)| FormatError(format!("hook {id:?}: {message}")))?;
        if hooks.insert(id.clone(), parsed).is_some() {
            return fail(format!("hook {id:?} is listed twice"));
        }
    }
    Ok(hooks)
}

fn convert(document: HookDocument) -> Result<Hook, FormatError> {
    if !is_valid_hook_id(&document.id) {
        return fail("the id must hold 1 to 64 characters from a-z, 0-9, '.', '_' and '-'");
    }
    let executable = PathBuf::from(&document.executable);
    check_executable_path(&executable)?;
    if document.args.iter().any(|argument| has_forbidden(argument)) {
        return fail("an argument holds a control character");
    }
    let timeout = document.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if timeout == 0 || timeout > MAX_TIMEOUT_SECONDS {
        return fail(format!(
            "timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
        ));
    }
    let working_dir = match &document.working_dir {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if !dir.is_absolute() || has_forbidden(&dir.to_string_lossy()) {
                return fail("working_dir must be an absolute path");
            }
            dir
        }
        None => executable
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| executable.clone()),
    };
    let mut phases = Vec::new();
    for phase in document.phases.unwrap_or_default() {
        let phase = match phase {
            PhaseDocument::Before => HookPhase::Before,
            PhaseDocument::After => HookPhase::After,
            PhaseDocument::OnSuccess => HookPhase::OnSuccess,
            PhaseDocument::OnError => HookPhase::OnError,
            PhaseDocument::Finally => HookPhase::Finally,
        };
        if !phases.contains(&phase) {
            phases.push(phase);
        }
    }
    let mut env = Vec::new();
    for (name, value) in &document.env {
        if !is_env_name(name) || has_forbidden(value) {
            return fail(format!("the environment variable {name:?} is not valid"));
        }
        env.push((name.clone(), value.clone()));
    }
    let parameters = parameters(&document.parameter, &env)?;
    Ok(Hook {
        id: document.id,
        executable,
        args: document.args,
        timeout: Duration::from_secs(timeout),
        working_dir,
        env,
        phases,
        parameters,
    })
}

/// An executable is an absolute path to a file. A batch file is refused on Windows, because the
/// command interpreter reads its own metacharacters in the arguments of a batch file.
fn check_executable_path(path: &Path) -> Result<(), FormatError> {
    if !path.is_absolute() || has_forbidden(&path.to_string_lossy()) {
        return fail("executable must be an absolute path");
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if cfg!(windows) && matches!(extension.as_deref(), Some("bat" | "cmd")) {
        return fail("a batch file cannot be a hook executable");
    }
    Ok(())
}

fn parameters(
    documents: &[ParameterDocument],
    fixed_env: &[(String, String)],
) -> Result<Vec<Parameter>, FormatError> {
    if documents.len() > MAX_PARAMETERS {
        return fail(format!("a hook has at most {MAX_PARAMETERS} parameters"));
    }
    let mut names = BTreeSet::new();
    let mut positions = BTreeSet::new();
    let mut env_names: BTreeSet<String> = fixed_env.iter().map(|(name, _)| name.clone()).collect();
    let mut parameters = Vec::new();
    for document in documents {
        if !is_valid_parameter_name(&document.name) {
            return fail(format!(
                "the parameter name {:?} must hold 1 to 64 characters from a-z, 0-9, '_' and '-'",
                document.name
            ));
        }
        if !names.insert(document.name.as_str()) {
            return fail(format!("the parameter {:?} is listed twice", document.name));
        }
        let parameter = parameter(document)?;
        match &parameter.mapping {
            Mapping::Positional(position) => {
                if !positions.insert(*position) {
                    return fail(format!("the position {position} is used twice"));
                }
            }
            Mapping::Env(name) => {
                if !env_names.insert(name.clone()) {
                    return fail(format!("the environment variable {name:?} is used twice"));
                }
            }
            Mapping::Flag => {}
        }
        parameters.push(parameter);
    }
    Ok(parameters)
}

fn parameter(document: &ParameterDocument) -> Result<Parameter, FormatError> {
    let name = &document.name;
    let unused = |present: bool, key: &str| -> Result<(), FormatError> {
        if present {
            fail(format!("{key} does not apply to the parameter {name:?}"))
        } else {
            Ok(())
        }
    };
    let kind = match document.kind {
        TypeDocument::String => {
            unused(
                document.minimum.is_some() || document.maximum.is_some(),
                "minimum and maximum",
            )?;
            unused(document.values.is_some(), "values")?;
            let max_length = document.max_length.unwrap_or(DEFAULT_MAX_LENGTH);
            if max_length == 0 || max_length > MAX_STRING_LENGTH {
                return fail(format!(
                    "max_length of {name:?} must be between 1 and {MAX_STRING_LENGTH}"
                ));
            }
            if let Some(pattern) = &document.pattern
                && (pattern.len() > 1024 || compile_pattern(pattern).is_err())
            {
                return fail(format!("the pattern of {name:?} is not valid"));
            }
            ParameterKind::String {
                max_length,
                pattern: document.pattern.clone(),
            }
        }
        TypeDocument::Integer => {
            unused(
                document.max_length.is_some() || document.pattern.is_some(),
                "max_length and pattern",
            )?;
            unused(document.values.is_some(), "values")?;
            if let (Some(minimum), Some(maximum)) = (document.minimum, document.maximum)
                && minimum > maximum
            {
                return fail(format!("minimum of {name:?} is above maximum"));
            }
            ParameterKind::Integer {
                minimum: document.minimum,
                maximum: document.maximum,
            }
        }
        TypeDocument::Enum => {
            unused(
                document.max_length.is_some() || document.pattern.is_some(),
                "max_length and pattern",
            )?;
            unused(
                document.minimum.is_some() || document.maximum.is_some(),
                "minimum and maximum",
            )?;
            let values = document.values.clone().unwrap_or_default();
            if values.is_empty()
                || values.len() > MAX_ENUM_VALUES
                || values
                    .iter()
                    .any(|value| value.is_empty() || value.len() > MAX_NAME || has_forbidden(value))
            {
                return fail(format!(
                    "values of {name:?} must hold 1 to {MAX_ENUM_VALUES} plain strings"
                ));
            }
            ParameterKind::Enum { values }
        }
        TypeDocument::Boolean => {
            unused(
                document.max_length.is_some() || document.pattern.is_some(),
                "max_length and pattern",
            )?;
            unused(
                document.minimum.is_some() || document.maximum.is_some(),
                "minimum and maximum",
            )?;
            unused(document.values.is_some(), "values")?;
            ParameterKind::Boolean
        }
        TypeDocument::SecretRef => {
            unused(
                document.max_length.is_some() || document.pattern.is_some(),
                "max_length and pattern",
            )?;
            unused(
                document.minimum.is_some() || document.maximum.is_some(),
                "minimum and maximum",
            )?;
            unused(document.values.is_some(), "values")?;
            ParameterKind::SecretRef
        }
    };

    let pass_as = document
        .pass_as
        .unwrap_or(if document.kind == TypeDocument::SecretRef {
            PassAs::Env
        } else {
            PassAs::Flag
        });
    let mapping = match (document.kind, pass_as) {
        // A secret never reaches an argument: the command line is visible to other users.
        (TypeDocument::SecretRef, PassAs::Env) => {
            unused(document.position.is_some(), "position")?;
            let Some(env_name) = &document.env_name else {
                return fail(format!("the secret parameter {name:?} needs env_name"));
            };
            if !is_env_name(env_name) || is_reserved_env(env_name) {
                return fail(format!(
                    "env_name {env_name:?} of {name:?} is not valid or is reserved"
                ));
            }
            Mapping::Env(env_name.clone())
        }
        (TypeDocument::SecretRef, _) => {
            return fail(format!(
                "the secret parameter {name:?} must use pass_as = \"env\""
            ));
        }
        (_, PassAs::Env) => {
            return fail(format!(
                "only a secret_ref parameter uses pass_as = \"env\" ({name:?})"
            ));
        }
        (_, PassAs::Flag) => {
            unused(document.position.is_some(), "position")?;
            unused(document.env_name.is_some(), "env_name")?;
            Mapping::Flag
        }
        (kind, PassAs::Positional) => {
            unused(document.env_name.is_some(), "env_name")?;
            if kind == TypeDocument::Boolean {
                return fail(format!(
                    "the boolean parameter {name:?} cannot be positional"
                ));
            }
            if document.required == Some(false) {
                return fail(format!(
                    "the positional parameter {name:?} must be required, so no value changes its slot"
                ));
            }
            match document.position {
                Some(position) if position >= 1 => Mapping::Positional(position),
                _ => {
                    return fail(format!(
                        "the positional parameter {name:?} needs position >= 1"
                    ));
                }
            }
        }
    };
    if !matches!(mapping, Mapping::Env(_)) && document.env_name.is_some() {
        return fail(format!("env_name does not apply to the parameter {name:?}"));
    }
    Ok(Parameter {
        name: name.clone(),
        spec: HookParameterSpec {
            kind,
            required: document.required.unwrap_or(true),
        },
        mapping,
    })
}

fn is_env_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    name.len() <= MAX_NAME
        && (first.is_ascii_uppercase() || first == '_')
        && characters.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

/// Variables that change how a program loads code or reads its own search path. A secret value
/// must not land in one of them.
fn is_reserved_env(name: &str) -> bool {
    name.starts_with("LD_")
        || name.starts_with("DYLD_")
        || matches!(
            name,
            "PATH" | "IFS" | "ENV" | "BASH_ENV" | "SHELLOPTS" | "BASHOPTS" | "PS4"
        )
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    const GOOD: &str = r#"
[[hook]]
id = "pg-dump"
executable = "/usr/local/lib/restichub/pg-dump"
args = ["--format=custom"]
timeout_seconds = 600
phases = ["before"]

[hook.env]
PGHOST = "/var/run/postgresql"

[[hook.parameter]]
name = "database"
type = "string"
max_length = 63
pattern = "[a-z_][a-z0-9_]*"
pass_as = "flag"

[[hook.parameter]]
name = "jobs"
type = "integer"
minimum = 1
maximum = 8
required = false

[[hook.parameter]]
name = "token"
type = "secret_ref"
env_name = "PGPASSWORD"
"#;

    #[test]
    fn fr_hk_02_a_complete_hook_parses() {
        let hooks = parse(GOOD).expect("the file is valid");
        let hook = hooks.get("pg-dump").expect("the hook is listed");
        assert_eq!(hook.timeout, Duration::from_secs(600));
        assert_eq!(hook.phases, vec![HookPhase::Before]);
        assert_eq!(hook.parameters.len(), 3);
        assert!(matches!(hook.parameters[2].mapping, Mapping::Env(_)));
        assert_eq!(hook.working_dir, PathBuf::from("/usr/local/lib/restichub"));
    }

    fn bad(text: &str) -> String {
        parse(text).expect_err("the file is not valid").to_string()
    }

    #[test]
    fn fr_hk_02_an_unknown_key_is_an_error() {
        let text = "[[hook]]\nid = \"a\"\nexecutable = \"/bin/true\"\ncommand = \"sh -c x\"\n";
        assert!(bad(text).contains("command"), "{}", bad(text));
        assert!(parse("shell = true\n").is_err());
    }

    #[test]
    fn fr_hk_02_a_relative_executable_and_a_bad_id_are_errors() {
        assert!(parse("[[hook]]\nid = \"a\"\nexecutable = \"bin/true\"\n").is_err());
        assert!(parse("[[hook]]\nid = \"A B\"\nexecutable = \"/bin/true\"\n").is_err());
        assert!(parse("[[hook]]\nid = \"a\"\nexecutable = \"/bin/true\"\n[[hook]]\nid = \"a\"\nexecutable = \"/bin/true\"\n").is_err());
    }

    #[test]
    fn fr_hk_02_a_secret_cannot_go_to_an_argument() {
        let base = "[[hook]]\nid = \"a\"\nexecutable = \"/bin/true\"\n[[hook.parameter]]\nname = \"t\"\ntype = \"secret_ref\"\n";
        assert!(parse(&format!("{base}pass_as = \"flag\"\n")).is_err());
        assert!(parse(&format!("{base}pass_as = \"positional\"\nposition = 1\n")).is_err());
        assert!(parse(&format!("{base}env_name = \"LD_PRELOAD\"\n")).is_err());
        assert!(parse(&format!("{base}env_name = \"PATH\"\n")).is_err());
        assert!(parse(&format!("{base}env_name = \"lower\"\n")).is_err());
        assert!(parse(base).is_err(), "env_name is required");
        assert!(parse(&format!("{base}env_name = \"TOKEN\"\n")).is_ok());
    }

    #[test]
    fn fr_hk_02_the_limits_of_a_parameter_are_checked() {
        let head =
            "[[hook]]\nid = \"a\"\nexecutable = \"/bin/true\"\n[[hook.parameter]]\nname = \"p\"\n";
        assert!(parse(&format!("{head}type = \"string\"\nmax_length = 5000\n")).is_err());
        assert!(parse(&format!("{head}type = \"string\"\npattern = \"(\"\n")).is_err());
        assert!(parse(&format!("{head}type = \"string\"\nminimum = 1\n")).is_err());
        assert!(
            parse(&format!(
                "{head}type = \"integer\"\nminimum = 5\nmaximum = 1\n"
            ))
            .is_err()
        );
        assert!(parse(&format!("{head}type = \"enum\"\n")).is_err());
        assert!(parse(&format!("{head}type = \"enum\"\nvalues = [\"a\"]\n")).is_ok());
        assert!(
            parse(&format!(
                "{head}type = \"boolean\"\npass_as = \"positional\"\nposition = 1\n"
            ))
            .is_err()
        );
        assert!(parse(&format!("{head}type = \"string\"\npass_as = \"positional\"\nposition = 1\nrequired = false\n")).is_err());
        assert!(
            parse(&format!(
                "{head}type = \"string\"\npass_as = \"positional\"\n"
            ))
            .is_err()
        );
    }

    #[test]
    fn fr_hk_02_the_timeout_has_a_range() {
        let text = |seconds: u64| {
            format!(
                "[[hook]]\nid = \"a\"\nexecutable = \"/bin/true\"\ntimeout_seconds = {seconds}\n"
            )
        };
        assert!(parse(&text(0)).is_err());
        assert!(parse(&text(86_401)).is_err());
        assert!(parse(&text(86_400)).is_ok());
    }
}
