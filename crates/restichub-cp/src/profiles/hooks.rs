//! The check of the hooks of a profile against the agents that report them (FR-HK-04, AC-HK-05).
//!
//! A profile may name a hook only when every target agent reports it with a schema that accepts the
//! values of the profile. The target agents of a stored profile are the agents of its plans. A
//! profile without a plan has no target yet, so each hook must exist on at least one active agent
//! and must fit every active agent that reports it; the check of the plan repeats the rule for the
//! agent of the plan.
//!
//! A parameter of the type `secret_ref` holds the identifier of a secret of the purpose
//! `hook_secret`, never a value (ADR-0017). The check refuses any other secret, so a profile cannot
//! hand a repository password to a hook.

use std::collections::BTreeSet;

use restichub_compiler::{
    CompileError, HookRef as CompilerHookRef, OperatingSystem, ResticVersion, TargetFacts,
    check_hook,
};
use restichub_db::agents::{self as agents_repo, Agent};
use restichub_db::{AgentStatus, ProfileId, SecretId, TenantTx, plans as plans_repo};
use restichub_secrets::SecretPurpose;
use time::OffsetDateTime;
use uuid::Uuid;

use super::settings::{HookRef, compiler_hook};
use super::{compile_error_parts, parse_arch, parse_labels, parse_os};
use crate::error::{ApiError, HookTarget};
use crate::validation::Reason;

/// Refuses a profile whose hooks do not fit its target agents (AC-HK-05).
pub(super) async fn check_profile_hooks(
    tx: &mut TenantTx<'_>,
    hooks: &[HookRef],
    profile: Option<ProfileId>,
    now: OffsetDateTime,
) -> Result<(), ApiError> {
    if hooks.is_empty() {
        return Ok(());
    }
    let compiler_hooks: Vec<CompilerHookRef> = hooks.iter().map(compiler_hook).collect();
    let mut planned: Vec<Agent> = Vec::new();
    if let Some(profile) = profile {
        let mut seen = BTreeSet::new();
        for plan in plans_repo::list_by_profile(tx, profile).await? {
            if !seen.insert(plan.agent_id.as_uuid()) {
                continue;
            }
            if let Some(agent) = agents_repo::get(tx, plan.agent_id).await?
                && agent.status == AgentStatus::Active
            {
                planned.push(agent);
            }
        }
    }

    let mut secrets: BTreeSet<(usize, usize)> = BTreeSet::new();
    if planned.is_empty() {
        let active = crate::hooks::active_agents(tx).await?;
        for (index, hook) in compiler_hooks.iter().enumerate() {
            let holders: Vec<&Agent> = active
                .iter()
                .filter(|agent| crate::hooks::allow_list_of(agent).contains_key(&hook.hook_id))
                .collect();
            if holders.is_empty() {
                return Err(ApiError::invalid_hook(
                    format!("hooks[{index}].hook_id"),
                    Reason::Unknown,
                    HookTarget {
                        agent_id: None,
                        agent_name: None,
                        hook_id: hook.hook_id.clone(),
                    },
                ));
            }
            for agent in holders {
                check_one(index, hook, agent, now, &mut secrets)?;
            }
        }
    } else {
        for agent in &planned {
            for (index, hook) in compiler_hooks.iter().enumerate() {
                check_one(index, hook, agent, now, &mut secrets)?;
            }
        }
    }

    for (index, position) in secrets {
        let field = format!("hooks[{index}].parameters[{position}].value");
        ensure_hook_secret(tx, &hooks[index].parameters[position].value, field).await?;
    }
    Ok(())
}

fn check_one(
    index: usize,
    hook: &CompilerHookRef,
    agent: &Agent,
    now: OffsetDateTime,
    secrets: &mut BTreeSet<(usize, usize)>,
) -> Result<(), ApiError> {
    let target = hook_target_facts(agent, now);
    check_hook(index, hook, &target).map_err(|error| hook_error(&error, hook, agent))?;
    if let Some(allowed) = target.allow_list.get(&hook.hook_id) {
        for (position, parameter) in hook.parameters.iter().enumerate() {
            if allowed
                .parameters
                .get(&parameter.name)
                .is_some_and(restichub_compiler::HookParameterSpec::secret)
            {
                secrets.insert((index, position));
            }
        }
    }
    Ok(())
}

/// The facts of an agent for the check of its hooks. A fact that the agent does not report in a
/// form that the compiler reads gets a neutral value, because the hook check needs only the
/// allow-list, the host name and the labels.
fn hook_target_facts(agent: &Agent, now: OffsetDateTime) -> TargetFacts {
    TargetFacts {
        agent_id: agent.id.to_string(),
        agent_name: agent.hostname.clone(),
        hostname: agent.hostname.clone(),
        os: parse_os(&agent.os).unwrap_or(OperatingSystem::Linux),
        arch: parse_arch(&agent.arch),
        labels: parse_labels(&agent.labels),
        restic_version: ResticVersion::parse(&agent.restic_version)
            .unwrap_or(ResticVersion::MIN_SUPPORTED),
        now,
        allow_list: crate::hooks::allow_list_of(agent),
        server_scripts: crate::scripts::server_scripts_of(agent),
    }
}

/// Turns a compiler refusal of a hook into an API error that names the agent and the hook.
pub(super) fn hook_error(error: &CompileError, hook: &CompilerHookRef, agent: &Agent) -> ApiError {
    let (field, reason) = compile_error_parts(error);
    ApiError::invalid_hook(
        field,
        reason,
        HookTarget {
            agent_id: Some(agent.id.to_string()),
            agent_name: Some(agent.hostname.clone()),
            hook_id: hook.hook_id.clone(),
        },
    )
}

/// The hook of the profile that a compiler error names, found by the position in its field.
pub(super) fn hook_of<'a>(
    error: &CompileError,
    hooks: &'a [CompilerHookRef],
) -> Option<&'a CompilerHookRef> {
    if !error.is_hook() {
        return None;
    }
    let field = error.field()?.as_str();
    let rest = field.strip_prefix("hooks[")?;
    let (index, _) = rest.split_once(']')?;
    hooks.get(index.parse::<usize>().ok()?)
}

/// Refuses a script variable that names a secret which is not a hook secret of the tenant. A
/// script receives the secret only through its environment, so it may not hand the script a
/// repository password (ADR-0017, FR-HK-09).
pub(crate) async fn check_script_secrets(
    tx: &mut TenantTx<'_>,
    scripts: &[crate::scripts::ScriptDef],
) -> Result<(), ApiError> {
    for (script, definition) in scripts.iter().enumerate() {
        for (variable, entry) in definition.env.iter().enumerate() {
            if entry.kind == crate::scripts::EnvKind::SecretRef && !entry.value.is_empty() {
                let field = format!("scripts[{script}].env[{variable}].value");
                ensure_hook_secret(tx, &entry.value, field).await?;
            }
        }
    }
    Ok(())
}

/// Refuses a value that is not the identifier of a hook secret of the tenant.
async fn ensure_hook_secret(
    tx: &mut TenantTx<'_>,
    value: &str,
    field: String,
) -> Result<(), ApiError> {
    let Ok(id) = Uuid::parse_str(value) else {
        return Err(ApiError::invalid_field(field, Reason::InvalidFormat));
    };
    let metadata = restichub_db::secrets::get_for_key_share(tx, SecretId::from_uuid(id)).await?;
    match metadata {
        Some(metadata) if metadata.purpose == SecretPurpose::HookSecret.as_str() => Ok(()),
        _ => Err(ApiError::invalid_field(field, Reason::Unknown)),
    }
}
