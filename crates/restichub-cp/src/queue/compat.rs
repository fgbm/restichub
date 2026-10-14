//! Fits the steps to the capabilities of the agent (FR-PR-09).
//!
//! An agent that does not know a field drops it without an error. A `forget` step that loses its
//! tag filter this way reaches every profile of the repository, so the builder never sends such a
//! step (FR-RP-08).

use restichub_proto::AgentCapability;
use restichub_proto::agent::v1::{Step, step};

use super::build::Refusal;

/// Moves a lone profile tag to the `tags` field, which every agent reads. A filter with more
/// than one tag needs `all_tags`, so only an agent with [`AgentCapability::ForgetAllTags`] gets it.
pub(super) fn fit_steps(
    steps: &mut [Step],
    capabilities: &[AgentCapability],
) -> Result<(), Refusal> {
    for step in steps {
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

/// The capabilities that the agent needs to read the steps without losing a field.
pub(super) fn required_capabilities(steps: &[Step]) -> Vec<AgentCapability> {
    let needs_all_tags = steps.iter().any(|step| {
        matches!(
            &step.kind,
            Some(step::Kind::Forget(forget))
                if forget.filter.as_ref().is_some_and(|filter| !filter.all_tags.is_empty())
        )
    });
    if needs_all_tags {
        vec![AgentCapability::ForgetAllTags]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::{ForgetStep, SnapshotsFilter};

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
}
