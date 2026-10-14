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
pub(super) fn fit_steps(steps: &mut [Step], capabilities: &[String]) -> Result<(), Refusal> {
    for step in steps {
        let Some(step::Kind::Forget(forget)) = step.kind.as_mut() else {
            continue;
        };
        let Some(filter) = forget.filter.as_mut() else {
            continue;
        };
        if filter.all_tags.len() == 1 && filter.tags.is_empty() {
            filter.tags = std::mem::take(&mut filter.all_tags);
        } else if !filter.all_tags.is_empty()
            && !AgentCapability::ForgetAllTags.is_listed(capabilities)
        {
            return Err(Refusal::AgentCapabilityMissing);
        }
    }
    Ok(())
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
        assert_eq!(
            fit_steps(&mut steps, &["backup".to_owned()]),
            Err(Refusal::AgentCapabilityMissing)
        );
    }

    #[test]
    fn fr_rp_08_an_agent_with_the_capability_gets_all_tags() {
        let mut steps = [forget(&["restichub-profile-a", "daily"])];
        let capabilities = [AgentCapability::ForgetAllTags.as_str().to_owned()];
        fit_steps(&mut steps, &capabilities).expect("a capable agent");
        assert_eq!(filter(&steps[0]).all_tags, ["restichub-profile-a", "daily"]);
        assert!(filter(&steps[0]).tags.is_empty());
    }
}
