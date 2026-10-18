//! Fits the steps to the capabilities of the agent (FR-PR-09).
//!
//! An agent that does not know a field drops it without an error. A `forget` step that loses its
//! tag filter this way reaches every profile of the repository, so the builder never sends such a
//! step (FR-RP-08). An agent without [`AgentCapability::RestoreV1`] gets no `ls` and no `restore`
//! step: an old agent rejects the kind, but the control plane never relies on that (FR-PR-09).

use restichub_proto::AgentCapability;
use restichub_proto::agent::v1::{Step, script_env, step};

use super::build::Refusal;

/// Moves a lone profile tag to the `tags` field, which every agent reads. A filter with more
/// than one tag needs `all_tags`, so only an agent with [`AgentCapability::ForgetAllTags`] gets it.
pub(super) fn fit_steps(
    steps: &mut [Step],
    capabilities: &[AgentCapability],
) -> Result<(), Refusal> {
    for step in steps {
        if matches!(step.kind, Some(step::Kind::Ls(_) | step::Kind::Restore(_)))
            && !capabilities.contains(&AgentCapability::RestoreV1)
        {
            return Err(Refusal::AgentCapabilityMissing);
        }
        // An agent that does not know the step would reject it, and the control plane never relies
        // on that (FR-HK-10).
        if matches!(step.kind, Some(step::Kind::Script(_)))
            && !capabilities.contains(&AgentCapability::ScriptsV1)
        {
            return Err(Refusal::ServerScriptsDisabled);
        }
        let Some(step::Kind::Forget(forget)) = step.kind.as_mut() else {
            continue;
        };
        let Some(filter) = forget.filter.as_mut() else {
            return Err(Refusal::ForgetWithoutTags);
        };
        if filter.all_tags.len() == 1 && filter.tags.is_empty() {
            filter.tags = std::mem::take(&mut filter.all_tags);
        } else if !filter.all_tags.is_empty()
            && !capabilities.contains(&AgentCapability::ForgetAllTags)
        {
            return Err(Refusal::AgentCapabilityMissing);
        }
        // A forget without any tag filter reaches every profile of the repository (FR-RP-08).
        if filter.tags.is_empty() && filter.all_tags.is_empty() {
            return Err(Refusal::ForgetWithoutTags);
        }
    }
    Ok(())
}

/// Replaces the parameter value of a script variable with a secret value for an agent that does not
/// know `masked_value` (FR-PF-12, ADR-0047).
///
/// An older agent rejects an unknown field of the oneof, and it masks a secret value in the step log
/// the same way. The call runs after [`resolve_hook_secrets`](super::build) resolved the references,
/// so a text value that reaches an old agent carries the same bytes and the same mask.
pub(super) fn fit_masked_values(steps: &mut [Step], capabilities: &[AgentCapability]) {
    if capabilities.contains(&AgentCapability::ScriptMaskedValue) {
        return;
    }
    for step in steps {
        let Some(step::Kind::Script(script)) = step.kind.as_mut() else {
            continue;
        };
        for variable in &mut script.env {
            if let Some(script_env::Value::MaskedValue(value)) = &mut variable.value {
                variable.value = Some(script_env::Value::SecretValue(std::mem::take(value)));
            }
        }
    }
}

/// The capabilities that the agent needs to read the steps without losing a field.
pub(super) fn required_capabilities(steps: &[Step]) -> Vec<AgentCapability> {
    let needs_masked_value = steps.iter().any(|step| {
        matches!(
            &step.kind,
            Some(step::Kind::Script(script))
                if script
                    .env
                    .iter()
                    .any(|variable| matches!(variable.value, Some(script_env::Value::MaskedValue(_))))
        )
    });
    let needs_all_tags = steps.iter().any(|step| {
        matches!(
            &step.kind,
            Some(step::Kind::Forget(forget))
                if forget.filter.as_ref().is_some_and(|filter| !filter.all_tags.is_empty())
        )
    });
    let needs_restore = steps
        .iter()
        .any(|step| matches!(step.kind, Some(step::Kind::Ls(_) | step::Kind::Restore(_))));
    let needs_scripts = steps
        .iter()
        .any(|step| matches!(step.kind, Some(step::Kind::Script(_))));
    let mut required = Vec::new();
    if needs_scripts {
        required.push(AgentCapability::ScriptsV1);
    }
    if needs_all_tags {
        required.push(AgentCapability::ForgetAllTags);
    }
    if needs_restore {
        required.push(AgentCapability::RestoreV1);
    }
    if needs_masked_value {
        required.push(AgentCapability::ScriptMaskedValue);
    }
    required
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::{
        ForgetStep, RestoreStep, ScriptEnv, ScriptStep, SnapshotsFilter, script_env,
    };

    use super::*;

    fn forget(all_tags: &[&str]) -> Step {
        Step {
            step_id: "forget".to_owned(),
            timeout_seconds: 0,
            kind: Some(step::Kind::Forget(ForgetStep {
                filter: Some(SnapshotsFilter {
                    all_tags: all_tags.iter().map(|tag| (*tag).to_owned()).collect(),
                    ..SnapshotsFilter::default()
                }),
                ..ForgetStep::default()
            })),
        }
    }

    fn filter(step: &Step) -> &SnapshotsFilter {
        let Some(step::Kind::Forget(forget)) = &step.kind else {
            panic!("a forget step");
        };
        forget.filter.as_ref().expect("a filter")
    }

    fn restore_step() -> Step {
        Step {
            step_id: "restore".to_owned(),
            timeout_seconds: 0,
            kind: Some(step::Kind::Restore(RestoreStep::default())),
        }
    }

    #[test]
    fn fr_pr_09_an_agent_without_restore_v1_never_gets_an_ls_or_restore_step() {
        let mut steps = [restore_step()];
        assert_eq!(
            fit_steps(&mut steps, &[AgentCapability::ForgetAllTags]),
            Err(Refusal::AgentCapabilityMissing)
        );
        assert_eq!(required_capabilities(&steps), [AgentCapability::RestoreV1]);
        fit_steps(&mut steps, &[AgentCapability::RestoreV1]).expect("a capable agent");
    }

    #[test]
    fn fr_rp_08_a_lone_profile_tag_goes_to_the_field_that_every_agent_reads() {
        let mut steps = [forget(&["restichub-profile-a"])];
        fit_steps(&mut steps, &[]).expect("an agent without capabilities");
        assert_eq!(filter(&steps[0]).tags, ["restichub-profile-a"]);
        assert!(filter(&steps[0]).all_tags.is_empty());
    }

    #[test]
    fn fr_rp_08_an_agent_without_the_capability_never_gets_all_tags() {
        let mut steps = [forget(&["restichub-profile-a", "daily"])];
        assert_eq!(
            fit_steps(&mut steps, &[]),
            Err(Refusal::AgentCapabilityMissing)
        );
    }

    #[test]
    fn fr_rp_08_an_agent_with_the_capability_gets_all_tags() {
        let mut steps = [forget(&["restichub-profile-a", "daily"])];
        fit_steps(&mut steps, &[AgentCapability::ForgetAllTags]).expect("a capable agent");
        assert_eq!(filter(&steps[0]).all_tags, ["restichub-profile-a", "daily"]);
        assert!(filter(&steps[0]).tags.is_empty());
    }

    #[test]
    fn fr_rp_08_a_forget_without_any_tag_filter_is_refused() {
        let mut steps = [forget(&[])];
        assert_eq!(
            fit_steps(&mut steps, &[AgentCapability::ForgetAllTags]),
            Err(Refusal::ForgetWithoutTags)
        );
    }

    #[test]
    fn fr_rp_08_a_forget_without_a_filter_is_refused() {
        let mut steps = [Step {
            step_id: "forget".to_owned(),
            timeout_seconds: 0,
            kind: Some(step::Kind::Forget(ForgetStep::default())),
        }];
        assert_eq!(
            fit_steps(&mut steps, &[AgentCapability::ForgetAllTags]),
            Err(Refusal::ForgetWithoutTags)
        );
    }

    fn script_step(value: script_env::Value) -> Step {
        Step {
            step_id: "script-0".to_owned(),
            timeout_seconds: 0,
            kind: Some(step::Kind::Script(ScriptStep {
                name: "dump".to_owned(),
                env: vec![ScriptEnv {
                    name: "PGDIR".to_owned(),
                    value: Some(value),
                }],
                ..ScriptStep::default()
            })),
        }
    }

    fn script_value(step: &Step) -> script_env::Value {
        let Some(step::Kind::Script(script)) = &step.kind else {
            panic!("a script step");
        };
        script.env[0].value.clone().expect("a value")
    }

    #[test]
    fn fr_pf_12_an_agent_without_the_capability_gets_a_secret_value() {
        let mut steps = [script_step(script_env::Value::MaskedValue(
            "/srv/pg".to_owned(),
        ))];
        assert_eq!(
            required_capabilities(&steps),
            [AgentCapability::ScriptMaskedValue]
        );
        fit_masked_values(&mut steps, &[]);
        assert_eq!(
            script_value(&steps[0]),
            script_env::Value::SecretValue("/srv/pg".to_owned())
        );
    }

    #[test]
    fn fr_pf_12_an_agent_with_the_capability_keeps_the_masked_value() {
        let mut steps = [script_step(script_env::Value::MaskedValue(
            "/srv/pg".to_owned(),
        ))];
        fit_masked_values(&mut steps, &[AgentCapability::ScriptMaskedValue]);
        assert_eq!(
            script_value(&steps[0]),
            script_env::Value::MaskedValue("/srv/pg".to_owned())
        );
        assert_eq!(required_capabilities(&steps), []);
    }
}
