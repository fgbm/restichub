//! The checks that keep a saved plan complete (FR-PF-11, AC-PF-06, AC-PF-07).
//!
//! Every write that can take a value away runs the check under the lock that the write holds: a
//! profile save locks the profile, a change of the host values locks the host, and a plan save
//! takes a share lock on the profile and a key share lock on the host. Thus a plan never reaches the
//! table without a value for each required parameter, and the scheduler has no incomplete plan to
//! dispatch.

use restichub_db::parameters as repo;
use restichub_db::{AgentId, ParameterKind, ProfileId};
use restichub_db::{plans as plans_repo, profiles as profiles_repo};

use crate::error::{ApiError, Blocker, BlockerKind, Blockers};
use crate::parameters::{self, ParameterDef};
use crate::validation::Reason;

/// The plans that a change would leave without a required value (FR-PF-11).
pub struct Missing {
    /// The names of the parameters that no source gives, without a repeat.
    pub names: Vec<String>,
    /// At most `REFERRER_LIMIT` plans, and the count of all of them.
    pub plans: Blockers,
}

/// The plans of one host that a missing required value leaves without a value.
pub async fn missing_in_plans_of_host(
    tx: &mut restichub_db::TenantTx<'_>,
    agent_id: AgentId,
) -> Result<Option<Missing>, ApiError> {
    let mut missing = Missing {
        names: Vec::new(),
        plans: Blockers {
            items: Vec::new(),
            total: 0,
        },
    };
    for plan in plans_repo::list_by_agent(tx, agent_id).await? {
        let Some(profile) = profiles_repo::get(tx, plan.profile_id).await? else {
            continue;
        };
        let definitions = parameters::from_row(&profile.parameters)?;
        let plan_values = repo::plan_values(tx, plan.id).await?;
        let host_values = repo::agent_values(tx, plan.agent_id).await?;
        add(
            &mut missing,
            parameters::missing_required(&definitions, &plan_values, &host_values),
            plan.id,
            &plan.name,
        );
    }
    Ok(finish(missing))
}

/// The plans of one profile that the declarations leave without a required value.
pub async fn missing_in_plans_of_profile(
    tx: &mut restichub_db::TenantTx<'_>,
    profile_id: ProfileId,
    definitions: &[ParameterDef],
) -> Result<Option<Missing>, ApiError> {
    let mut missing = Missing {
        names: Vec::new(),
        plans: Blockers {
            items: Vec::new(),
            total: 0,
        },
    };
    for plan in plans_repo::list_by_profile(tx, profile_id).await? {
        let plan_values = repo::plan_values(tx, plan.id).await?;
        let host_values = repo::agent_values(tx, plan.agent_id).await?;
        add(
            &mut missing,
            parameters::missing_required(definitions, &plan_values, &host_values),
            plan.id,
            &plan.name,
        );
    }
    Ok(finish(missing))
}

fn add(missing: &mut Missing, names: Vec<String>, plan_id: restichub_db::PlanId, plan_name: &str) {
    if names.is_empty() {
        return;
    }
    missing.plans.total += 1;
    if missing.plans.items.len() < restichub_db::referrers::REFERRER_LIMIT as usize {
        missing.plans.items.push(Blocker {
            kind: BlockerKind::Plan,
            id: plan_id.to_string(),
            name: plan_name.to_owned(),
        });
    }
    for name in names {
        if !missing.names.contains(&name) {
            missing.names.push(name);
        }
    }
}

fn finish(missing: Missing) -> Option<Missing> {
    (missing.plans.total > 0).then_some(missing)
}

/// Refuses a change that leaves plans without a required value (FR-PF-11).
///
/// # Errors
///
/// Returns `400 parameter_missing_in_plans` with the names of the parameters and the plans.
pub fn refuse_missing(missing: Missing) -> ApiError {
    ApiError::parameters_in_plans(
        Reason::ParameterMissingInPlans,
        missing.names,
        missing.plans,
    )
}

/// Refuses a submitted value whose name no declaration holds or whose kind differs (FR-PF-11).
///
/// # Errors
///
/// Returns a field error for an undeclared name and `parameter_kind_mismatch` for a kind.
pub fn check_declared_values(
    definitions: &[ParameterDef],
    values: &[repo::ParameterValue],
    field: &str,
) -> Result<(), ApiError> {
    if let Some(name) = parameters::undeclared(definitions, values).first() {
        return Err(ApiError::invalid_field_cause(
            field,
            Reason::Unknown,
            format!("the parameter {name} is not declared"),
        ));
    }
    let mismatched = parameters::kind_mismatches(definitions, values);
    if !mismatched.is_empty() {
        return Err(ApiError::parameters(
            field,
            Reason::ParameterKindMismatch,
            mismatched,
        ));
    }
    Ok(())
}

/// Refuses a secret value that names no secret of the purpose `hook_secret` (ADR-0047).
///
/// # Errors
///
/// Returns a field error for an unknown secret and an unsupported purpose.
pub async fn check_secret_values(
    tx: &mut restichub_db::TenantTx<'_>,
    values: &[repo::ParameterValue],
    field: &str,
) -> Result<(), ApiError> {
    for value in values
        .iter()
        .filter(|value| value.kind == ParameterKind::Secret)
    {
        let Some(id) = value.secret_id else {
            continue;
        };
        // The key share lock makes a concurrent delete of the secret wait for this transaction,
        // so the delete sees the reference and refuses.
        let Some(metadata) = restichub_db::secrets::get_for_key_share(tx, id).await? else {
            return Err(ApiError::invalid_field(field, Reason::Unknown));
        };
        if metadata.purpose != restichub_secrets::SecretPurpose::HookSecret.as_str() {
            return Err(ApiError::invalid_field(field, Reason::Unsupported));
        }
    }
    Ok(())
}
