//! The profile parameters and their values (FR-PF-09 to FR-PF-12, ADR-0047).
//!
//! A profile declares parameters: a name, a kind, a required flag, a default value for a `text`
//! parameter and a description. The declaration is part of the profile JSON, so the profile carries
//! its own contract and a rename becomes a new parameter.
//!
//! A host holds one value for each parameter name and a plan overrides a host value. The value
//! resolves in the order plan, host, default. A `secret` value is the identifier of a secret of the
//! purpose `hook_secret`; the control plane stores no secret text in a value (ADR-0017).
//!
//! A `text` value expands in the paths, the excludes, the exclude files and the tags (FR-PF-10). A
//! `secret` value never expands, because restic arguments are visible in the process list
//! (FR-AG-06). A value never appears in an audit diff (FR-PF-12).

mod checks;
mod host;

use std::collections::BTreeMap;

pub use checks::{
    check_declared_values, check_kinds_after_move, check_kinds_of_host, check_secret_values,
    missing_in_plans_of_host, missing_in_plans_of_profile, refuse_missing,
};
pub use host::*;

use restichub_compiler::ParameterValue as CompilerParameterValue;
use restichub_db::ParameterKind;
use restichub_db::parameters::ParameterValue;
use restichub_proto::validate::has_forbidden;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::validation::Reason;

/// Limit of the parameters of one profile.
pub const MAX_PARAMETERS: usize = 64;
/// Limit in bytes of one parameter value and of one default value. It stays below the limit of the
/// value of a script variable, which the agent checks (FR-PF-10).
pub const MAX_PARAMETER_VALUE: usize = 4096;
/// Limit in characters of the description of a parameter.
pub const MAX_PARAMETER_DESCRIPTION: usize = 200;
/// The longest parameter name, without the leading letter.
const MAX_PARAMETER_NAME_SUFFIX: usize = 63;

/// The scope of the advisory lock that serializes the writes which change what the plans of a
/// tenant need: the declarations of a profile, the values of a host, the profile and the host of a
/// plan (FR-PF-11, ADR-0047).
///
/// [`restichub_db::TenantTx::lock`] derives the 64-bit key as `hashtextextended(scope || tenant, 0)`.
/// The scope is the fixed namespace of this invariant, and the tenant identifier makes the key of
/// one tenant distinct from the key of another. PostgreSQL compares the key across every
/// connection, so the lock also serializes two control plane instances. A write takes it before any
/// row lock, so a reader of the invariant cannot wait for a row that another writer holds.
pub const PARAMETERS_LOCK: &str = "restichub.parameters:";

/// The declaration of one parameter as the profile row stores it (FR-PF-09).
///
/// The storage form tolerates a field that this version does not know, so a newer row does not stop
/// an older control plane. [`ParameterDefRequest`] is the strict form of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ParameterDef {
    /// A lower-case letter, then letters, digits and underscores; at most 64 characters.
    #[schema(pattern = "^[a-z][a-z0-9_]{0,63}$")]
    pub name: String,
    pub kind: ParameterKind,
    /// A plan without a value for a required parameter is refused (FR-PF-11).
    #[serde(default)]
    pub required: bool,
    /// A text parameter only. A secret default would give one password to every host (ADR-0047).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// At most 200 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The declaration of one parameter in a profile request (FR-PF-09).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ParameterDefRequest {
    /// A lower-case letter, then letters, digits and underscores; at most 64 characters.
    #[schema(pattern = "^[a-z][a-z0-9_]{0,63}$")]
    pub name: String,
    pub kind: ParameterKind,
    /// A plan without a value for a required parameter is refused (FR-PF-11).
    #[serde(default)]
    pub required: bool,
    /// A text parameter only. A secret default would give one password to every host (ADR-0047).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// At most 200 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl From<ParameterDefRequest> for ParameterDef {
    fn from(request: ParameterDefRequest) -> Self {
        Self {
            name: request.name,
            kind: request.kind,
            required: request.required,
            default_value: request.default_value,
            description: request.description,
        }
    }
}

/// One value of a parameter in a plan request or in a host request (FR-PF-10).
///
/// The kind must agree with the declaration. Exactly one of `value` and `secret_id` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ParameterValueDef {
    pub name: String,
    pub kind: ParameterKind,
    /// A text of 1 to 4096 bytes, for a `text` parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The identifier of a secret of the purpose `hook_secret`, for a `secret` parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_id: Option<String>,
}

/// The declaration of one parameter as a reader sees it (FR-PF-09, FR-PF-12).
///
/// The type is separate from [`ParameterDef`] because it holds the marker of a hidden value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ParameterDefView {
    pub name: String,
    pub kind: ParameterKind,
    pub required: bool,
    /// What a text parameter falls back to. Empty with the flag `hidden` for a Viewer.
    pub default_value: Option<String>,
    pub description: Option<String>,
    /// True when the server hid the default value from this reader (FR-PF-12).
    pub hidden: bool,
}

impl ParameterDefView {
    /// `may_read_text` is false for a Viewer, which reads no text value (FR-PF-12).
    #[must_use]
    pub fn of(definition: &ParameterDef, may_read_text: bool) -> Self {
        let hidden = !may_read_text
            && definition.kind == ParameterKind::Text
            && definition.default_value.is_some();
        Self {
            name: definition.name.clone(),
            kind: definition.kind,
            required: definition.required,
            default_value: match (&definition.default_value, may_read_text) {
                (Some(_), false) => Some(String::new()),
                (value, _) => value.clone(),
            },
            description: definition.description.clone(),
            hidden,
        }
    }
}

/// One override of a plan as a reader sees it (FR-PF-10, FR-PF-12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PlanParameterValueView {
    pub name: String,
    pub kind: ParameterKind,
    /// The text as the reader sees it: empty with the flag `hidden` for a Viewer (FR-PF-12).
    pub value: Option<String>,
    /// The identifier of the secret of a secret parameter.
    pub secret_id: Option<String>,
    pub secret_name: Option<String>,
    /// True when the server hid a text value from this reader (FR-PF-12).
    pub hidden: bool,
}

impl PlanParameterValueView {
    /// `may_read_text` is false for a Viewer, which reads no text value (FR-PF-12).
    #[must_use]
    pub fn of(stored: &restichub_db::parameters::PlanValue, may_read_text: bool) -> Self {
        let hidden = !may_read_text
            && stored.value.kind == ParameterKind::Text
            && stored.value.value.is_some();
        Self {
            name: stored.value.name.clone(),
            kind: stored.value.kind,
            value: match (&stored.value.value, may_read_text) {
                (Some(_), false) => Some(String::new()),
                (value, _) => value.clone(),
            },
            secret_id: stored.value.secret_id.map(|id| id.to_string()),
            secret_name: stored.secret_name.clone(),
            hidden,
        }
    }
}

/// The resolved values of a stored plan for the compiler (FR-PF-10).
///
/// # Errors
///
/// Returns an internal error for a stored row that this version cannot read.
pub async fn resolved_for_plan(
    tx: &mut restichub_db::TenantTx<'_>,
    definitions: &[ParameterDef],
    plan_id: restichub_db::PlanId,
    agent_id: restichub_db::AgentId,
) -> Result<BTreeMap<String, CompilerParameterValue>, ApiError> {
    let plan_values = restichub_db::parameters::plan_values(tx, plan_id).await?;
    let host_values = restichub_db::parameters::agent_values(tx, agent_id).await?;
    Ok(resolve(definitions, &plan_values, &host_values))
}

/// True for a role that may read a text value and a text default (FR-PF-12, FR-HK-12).
#[must_use]
pub fn may_read_text(context: &RequestContext) -> bool {
    context.authorize(Operation::ScriptsRead).is_ok()
}

/// True for `^[a-z][a-z0-9_]{0,63}$` (FR-PF-09).
#[must_use]
pub fn is_parameter_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && name.len() <= MAX_PARAMETER_NAME_SUFFIX + 1
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

/// Validates the declarations of a profile and returns them normalized (FR-PF-09).
///
/// # Errors
///
/// Returns a field error for a name, a count, a default value or a description that the decision
/// refuses.
pub fn validate_definitions(
    field: &str,
    definitions: Vec<ParameterDefRequest>,
) -> Result<Vec<ParameterDef>, ApiError> {
    if definitions.len() > MAX_PARAMETERS {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut result = Vec::with_capacity(definitions.len());
    for (index, definition) in definitions.into_iter().enumerate() {
        let at = format!("{field}[{index}]");
        let definition = ParameterDef::from(definition);
        if !is_parameter_name(&definition.name) {
            return Err(ApiError::invalid_field(
                format!("{at}.name"),
                Reason::InvalidFormat,
            ));
        }
        if !names.insert(definition.name.clone()) {
            return Err(ApiError::invalid_field(
                format!("{at}.name"),
                Reason::Duplicate,
            ));
        }
        let default_value = match (definition.kind, definition.default_value) {
            (ParameterKind::Text, Some(value)) => {
                Some(validate_value(&format!("{at}.default_value"), &value)?)
            }
            // Only a text parameter has a default (ADR-0047).
            (ParameterKind::Secret, Some(_)) => {
                return Err(ApiError::invalid_field(
                    format!("{at}.default_value"),
                    Reason::Unsupported,
                ));
            }
            (_, None) => None,
        };
        let description = match definition.description {
            Some(value) => {
                let value = value.trim().to_owned();
                if value.chars().count() > MAX_PARAMETER_DESCRIPTION {
                    return Err(ApiError::invalid_field(
                        format!("{at}.description"),
                        Reason::TooLong,
                    ));
                }
                if has_forbidden(&value) {
                    return Err(ApiError::invalid_field(
                        format!("{at}.description"),
                        Reason::InvalidCharacter,
                    ));
                }
                Some(value)
            }
            None => None,
        };
        result.push(ParameterDef {
            name: definition.name,
            kind: definition.kind,
            required: definition.required,
            default_value,
            description,
        });
    }
    Ok(result)
}

/// A text value: 1 to 4096 bytes and no control or format character.
fn validate_value(field: &str, value: &str) -> Result<String, ApiError> {
    if value.is_empty() {
        return Err(ApiError::invalid_field(field, Reason::Empty));
    }
    if value.len() > MAX_PARAMETER_VALUE {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    if has_forbidden(value) {
        return Err(ApiError::invalid_field(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

/// Validates the values of a plan request or of a host request (FR-PF-10).
///
/// # Errors
///
/// Returns a field error for a name, a count or a value that breaks the kind.
pub fn validate_values(
    field: &str,
    values: Vec<ParameterValueDef>,
) -> Result<Vec<ParameterValue>, ApiError> {
    if values.len() > MAX_PARAMETERS {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut result = Vec::with_capacity(values.len());
    for (index, value) in values.into_iter().enumerate() {
        let at = format!("{field}[{index}]");
        if !is_parameter_name(&value.name) {
            return Err(ApiError::invalid_field(
                format!("{at}.name"),
                Reason::InvalidFormat,
            ));
        }
        if !names.insert(value.name.clone()) {
            return Err(ApiError::invalid_field(
                format!("{at}.name"),
                Reason::Duplicate,
            ));
        }
        let stored = match (value.kind, value.value, value.secret_id) {
            (ParameterKind::Text, Some(text), None) => ParameterValue {
                name: value.name,
                kind: ParameterKind::Text,
                value: Some(validate_value(&format!("{at}.value"), &text)?),
                secret_id: None,
            },
            (ParameterKind::Secret, None, Some(id)) => {
                let id = uuid::Uuid::parse_str(id.trim()).map_err(|error| {
                    ApiError::invalid_field_cause(
                        format!("{at}.secret_id"),
                        Reason::InvalidFormat,
                        error.to_string(),
                    )
                })?;
                ParameterValue {
                    name: value.name,
                    kind: ParameterKind::Secret,
                    value: None,
                    secret_id: Some(restichub_db::SecretId::from_uuid(id)),
                }
            }
            // A text value of a secret parameter, a secret reference of a text parameter and an
            // entry with no value would hide the kind from the resolution.
            (ParameterKind::Text, None, _) => {
                return Err(ApiError::invalid_field(
                    format!("{at}.value"),
                    Reason::Empty,
                ));
            }
            (ParameterKind::Secret, None, None) => {
                return Err(ApiError::invalid_field(
                    format!("{at}.secret_id"),
                    Reason::Empty,
                ));
            }
            (ParameterKind::Text, _, _) => {
                return Err(ApiError::invalid_field(
                    format!("{at}.secret_id"),
                    Reason::Unsupported,
                ));
            }
            (ParameterKind::Secret, _, _) => {
                return Err(ApiError::invalid_field(
                    format!("{at}.value"),
                    Reason::Unsupported,
                ));
            }
        };
        result.push(stored);
    }
    Ok(result)
}

/// Reads the declarations of a stored profile. A row that does not parse is an internal error.
///
/// # Errors
///
/// Returns an internal error for a row that this version cannot read.
pub fn from_row(value: &Value) -> Result<Vec<ParameterDef>, ApiError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        ApiError::internal(format!("the stored parameters are not valid: {error}"))
    })
}

/// The declarations as the profile row stores them.
#[must_use]
pub fn to_value(definitions: &[ParameterDef]) -> Value {
    serde_json::to_value(definitions).unwrap_or_else(|_| json!([]))
}

/// The declarations of a stored row for an audit diff. A row that does not parse shows as null.
#[must_use]
pub fn stored_value(value: &Value) -> Value {
    from_row(value).map_or(Value::Null, |definitions| {
        audit_definitions(&definitions, None)
    })
}

/// The declarations for an audit diff: the names, the kinds, the descriptions and no text default
/// (FR-PF-12).
///
/// `previous` is the other side of the diff. The marker of a default value is [`CHANGED`] when the
/// text of the previous declaration differs, so a change of a default shows in the diff and an
/// unchanged one does not. The diff holds the marker only, never the text. The declarations are
/// sorted by name, so a reorder of the same set is no change.
#[must_use]
pub fn audit_definitions(definitions: &[ParameterDef], previous: Option<&[ParameterDef]>) -> Value {
    let mut sorted: Vec<&ParameterDef> = definitions.iter().collect();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    Value::Array(
        sorted
            .iter()
            .map(|definition| {
                let before = previous.and_then(|previous| {
                    previous
                        .iter()
                        .find(|other| other.name == definition.name && other.kind == definition.kind)
                });
                json!({
                    "name": definition.name,
                    "kind": definition.kind.as_str(),
                    "required": definition.required,
                    "default_value": definition
                        .default_value
                        .as_ref()
                        .map(|text| marker(before.and_then(|definition| definition.default_value.as_deref()), text)),
                    "description": definition
                        .description
                        .as_ref()
                        .map(|text| marker(before.and_then(|definition| definition.description.as_deref()), text)),
                })
            })
            .collect(),
    )
}

/// The values of a record for an audit diff: the names, the kinds, the reference of a secret value
/// and a marker for a text value (FR-PF-12).
///
/// `previous` is the side of the diff that the write replaces. See [`audit_definitions`]. Both sides
/// are sorted by name, so a set in another order is no change.
///
/// The reference is a key name without `secret` in it: the diff of the audit log redacts every
/// field whose name holds that word, and an identifier is no secret.
#[must_use]
pub fn audit_values(values: &[ParameterValue], previous: Option<&[ParameterValue]>) -> Value {
    let mut sorted: Vec<&ParameterValue> = values.iter().collect();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    Value::Array(
        sorted
            .iter()
            .map(|value| {
                let before = previous.and_then(|previous| {
                    previous
                        .iter()
                        .find(|other| other.name == value.name && other.kind == value.kind)
                });
                json!({
                    "name": value.name,
                    "kind": value.kind.as_str(),
                    "value": value
                        .value
                        .as_ref()
                        .map(|text| marker(before.and_then(|value| value.value.as_deref()), text)),
                    "reference": value.secret_id.map(|id| id.to_string()),
                })
            })
            .collect(),
    )
}

/// The overrides that a profile save removes, for an audit diff (FR-PF-12).
///
/// The diff names the plan, the parameter and the reference of a secret value; a text value shows as
/// the marker of [`audit_values`] only.
#[must_use]
pub fn audit_dropped(dropped: &[(restichub_db::PlanId, ParameterValue)]) -> Value {
    Value::Array(
        dropped
            .iter()
            .map(|(plan_id, value)| {
                json!({
                    "plan_id": plan_id.to_string(),
                    "name": value.name,
                    "kind": value.kind.as_str(),
                    "reference": value.secret_id.map(|id| id.to_string()),
                })
            })
            .collect(),
    )
}

/// True when the write adds, removes or changes a value of the kind `secret` (ADR-0047).
///
/// Only an Admin changes such a value: it chooses the credential that a script receives. An entry
/// that the body leaves as it was is no change, so an Operator can save a full set that still names
/// the secret of an Admin.
#[must_use]
pub fn changes_secret(before: &[ParameterValue], after: &[ParameterValue]) -> bool {
    fn secrets(values: &[ParameterValue]) -> BTreeMap<&str, Option<restichub_db::SecretId>> {
        values
            .iter()
            .filter(|value| value.kind == ParameterKind::Secret)
            .map(|value| (value.name.as_str(), value.secret_id))
            .collect()
    }
    secrets(before) != secrets(after)
}

/// The marker of a text value that the diff names but does not show.
pub const CHANGED: &str = "<changed>";
/// The marker of a text value that the write sets to the value that the other side holds.
pub const SET: &str = "<set>";

fn marker(before: Option<&str>, after: &str) -> &'static str {
    match before {
        Some(before) if before != after => CHANGED,
        _ => SET,
    }
}

/// Resolves the value of each declared parameter for one plan (FR-PF-10).
///
/// The order is the plan, the host, then the default. A value of another kind than the declaration
/// does not count. An optional parameter of either kind without a value resolves to the empty
/// string, as the owner decided, so a script variable of it is set and empty; a required one stays
/// absent, and the completeness check refuses it.
#[must_use]
pub fn resolve(
    definitions: &[ParameterDef],
    plan_values: &[ParameterValue],
    host_values: &[ParameterValue],
) -> BTreeMap<String, CompilerParameterValue> {
    let mut resolved = BTreeMap::new();
    for definition in definitions {
        let found = plan_values
            .iter()
            .chain(host_values)
            .find(|value| value.name == definition.name && value.kind == definition.kind);
        match (found, definition.kind, definition.default_value.as_deref()) {
            (Some(value), ParameterKind::Text, _) => {
                if let Some(text) = &value.value {
                    resolved.insert(
                        definition.name.clone(),
                        CompilerParameterValue::Text(text.clone()),
                    );
                }
            }
            (Some(value), ParameterKind::Secret, _) => {
                if let Some(id) = value.secret_id {
                    resolved.insert(
                        definition.name.clone(),
                        CompilerParameterValue::Secret(id.to_string()),
                    );
                }
            }
            (None, ParameterKind::Text, Some(default)) => {
                resolved.insert(
                    definition.name.clone(),
                    CompilerParameterValue::Text(default.to_owned()),
                );
            }
            (None, _, _) if !definition.required => {
                resolved.insert(
                    definition.name.clone(),
                    CompilerParameterValue::Text(String::new()),
                );
            }
            (None, _, _) => {}
        }
    }
    resolved
}

/// The names of the required parameters that no source gives a value (FR-PF-11).
///
/// A value of another kind than the declaration counts as missing.
#[must_use]
pub fn missing_required(
    definitions: &[ParameterDef],
    plan_values: &[ParameterValue],
    host_values: &[ParameterValue],
) -> Vec<String> {
    let mut missing = Vec::new();
    for definition in definitions.iter().filter(|definition| definition.required) {
        let found = plan_values
            .iter()
            .chain(host_values)
            .any(|value| value.name == definition.name && value.kind == definition.kind);
        let default = definition.kind == ParameterKind::Text && definition.default_value.is_some();
        if !found && !default {
            missing.push(definition.name.clone());
        }
    }
    missing
}

/// The names of the submitted values whose kind differs from the declaration (FR-PF-11).
#[must_use]
pub fn kind_mismatches(definitions: &[ParameterDef], values: &[ParameterValue]) -> Vec<String> {
    values
        .iter()
        .filter(|value| {
            definitions
                .iter()
                .any(|definition| definition.name == value.name && definition.kind != value.kind)
        })
        .map(|value| value.name.clone())
        .collect()
}

/// The names of the submitted values that the declarations do not hold.
#[must_use]
pub fn undeclared<'a>(definitions: &[ParameterDef], values: &'a [ParameterValue]) -> Vec<&'a str> {
    values
        .iter()
        .filter(|value| {
            !definitions
                .iter()
                .any(|definition| definition.name == value.name)
        })
        .map(|value| value.name.as_str())
        .collect()
}

/// Checks the parameter references of the template fields of a profile before a save
/// (FR-PF-05, FR-PF-10).
///
/// A secret parameter cannot expand, because the restic arguments are visible (FR-AG-06). An
/// optional text parameter without a default is the empty string, and an empty path, exclude or tag
/// is invalid, so the profile does not save.
///
/// # Errors
///
/// Returns a field error for the first reference that the save cannot accept.
pub fn check_templates<'a>(
    definitions: &[ParameterDef],
    fields: impl IntoIterator<Item = (&'a str, &'a String)>,
) -> Result<(), ApiError> {
    for (field, value) in fields {
        for name in restichub_compiler::parameter_names(value) {
            let Some(definition) = definitions
                .iter()
                .find(|definition| definition.name == name)
            else {
                return Err(ApiError::invalid_field(field, Reason::Unknown));
            };
            if definition.kind == ParameterKind::Secret {
                return Err(ApiError::invalid_field(
                    field,
                    Reason::ParameterSecretInTemplate,
                ));
            }
            if !definition.required && definition.default_value.is_none() {
                return Err(ApiError::invalid_field(field, Reason::ParameterMissing));
            }
        }
    }
    Ok(())
}

/// Refuses a parameter in the retention or in the check (ADR-0034, ADR-0047).
///
/// One maintenance step serves every host of the plans of a repository, so a value of one host has
/// no single meaning there.
///
/// # Errors
///
/// The error is `parameter_not_allowed` on that field.
pub fn check_maintenance(value: &str, field: &str) -> Result<(), ApiError> {
    match restichub_compiler::parameter_names(value).first() {
        Some(name) => Err(ApiError::invalid_field_cause(
            field,
            Reason::ParameterNotAllowed,
            format!("the parameter {name} cannot expand in this field"),
        )),
        None => Ok(()),
    }
}

/// Checks that each parameter variable of a script names a declared parameter (FR-PF-05).
///
/// # Errors
///
/// Returns a field error on the variable that names a parameter that the profile does not declare.
pub fn check_script_parameters(
    definitions: &[ParameterDef],
    scripts: &[crate::scripts::ScriptDef],
) -> Result<(), ApiError> {
    for (index, script) in scripts.iter().enumerate() {
        for (position, variable) in script.env.iter().enumerate() {
            if variable.kind != crate::scripts::EnvKind::Parameter {
                continue;
            }
            if !definitions
                .iter()
                .any(|definition| definition.name == variable.value)
            {
                return Err(ApiError::invalid_field(
                    format!("scripts[{index}].env[{position}].value"),
                    Reason::Unknown,
                ));
            }
        }
    }
    Ok(())
}
