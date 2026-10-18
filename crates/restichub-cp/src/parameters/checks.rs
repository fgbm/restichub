//! The checks that keep a saved plan complete (FR-PF-11, AC-PF-06, AC-PF-07).
//!
//! Every write that can take a value away takes one transaction-level advisory lock of the tenant
//! first: `pg_advisory_xact_lock` on the key of [`crate::parameters::PARAMETERS_LOCK`] and the
//! tenant. The lock covers a profile create, update and delete, a plan create, update and delete, a
//! change of the host values and the replace of a host record (FR-ID-11). A plan therefore never
//! reaches the table without a value for each required parameter, and the scheduler has no
//! incomplete plan to dispatch.
//!
//! The advisory lock comes before every row lock, and it serializes the writers of the tenant, so
//! the checks below read the rows of the other writers without a share lock. The order of the row
//! locks after the advisory lock is this one, and no path takes them in the other order:
//!
//! - a plan save: plan, profile, host, repository;
//! - a host values change: host, then the profiles of the plans of the host;
//! - a profile save: the profile only.
//!
//! The repository row keeps its update lock, because ADR-0046 binds a local repository to its host
//! and the repository write takes that lock first. Two writers of this invariant wait for the
//! advisory lock, so no row lock of the invariant forms a cycle.

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
///
/// The caller holds the advisory lock of the parameter writes, so a concurrent profile save waits
/// for this transaction (FR-PF-11).
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

/// Refuses a declaration whose name another profile of the same host declares with another kind
/// (FR-PF-11, ADR-0047).
///
/// A host holds one value for each name, so two kinds for one name on one host cannot both have a
/// value. `own` is the profile that the write saves, and the plans of the other profiles of the
/// host give their declarations. `skip_plan` is the plan that the write saves: its stored row still
/// names its old profile, and that profile is not another profile of the host (FR-ID-11).
///
/// # Errors
///
/// Returns `400 parameter_kind_mismatch` with the name of the parameter.
pub async fn check_kinds_of_host(
    tx: &mut restichub_db::TenantTx<'_>,
    agent_id: AgentId,
    own: ProfileId,
    skip_plan: Option<restichub_db::PlanId>,
    field: &str,
    definitions: &[ParameterDef],
) -> Result<(), ApiError> {
    for plan in plans_repo::list_by_agent(tx, agent_id).await? {
        if plan.profile_id == own || Some(plan.id) == skip_plan {
            continue;
        }
        let Some(profile) = profiles_repo::get(tx, plan.profile_id).await? else {
            continue;
        };
        for other in parameters::from_row(&profile.parameters)? {
            let shared = definitions
                .iter()
                .any(|definition| definition.name == other.name);
            if shared && other.kind != kind_of(definitions, &other.name) {
                return Err(ApiError::parameter_kind_conflict(
                    field,
                    other.name,
                    &profile.name,
                ));
            }
        }
    }
    Ok(())
}

fn kind_of(definitions: &[ParameterDef], name: &str) -> ParameterKind {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .map_or(ParameterKind::Text, |definition| definition.kind)
}

/// Refuses a move of plans to a host that declares one name with another kind (FR-PF-11, FR-ID-11).
///
/// The move gives the plans of the old record the successor, and the values of the successor then
/// serve their profiles too. The check runs after the move, so the plans of the successor supply
/// the declarations of both records.
///
/// # Errors
///
/// Returns `400 parameter_kind_mismatch` with the name of the parameter.
pub async fn check_kinds_after_move(
    tx: &mut restichub_db::TenantTx<'_>,
    agent_id: AgentId,
) -> Result<(), ApiError> {
    let mut checked: Vec<ProfileId> = Vec::new();
    for plan in plans_repo::list_by_agent(tx, agent_id).await? {
        if checked.contains(&plan.profile_id) {
            continue;
        }
        checked.push(plan.profile_id);
        let Some(profile) = profiles_repo::get(tx, plan.profile_id).await? else {
            continue;
        };
        let definitions = parameters::from_row(&profile.parameters)?;
        check_kinds_of_host(
            tx,
            agent_id,
            plan.profile_id,
            None,
            "old_agent_id",
            &definitions,
        )
        .await?;
    }
    Ok(())
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
