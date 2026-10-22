use restichub_proto::agent::v1::{ProcessPriority, step};

use super::tests::{plan, profile, target};
use crate::profile::{CheckOptions, Priority, Profile, Retention};

fn priorities(profile: &Profile) -> Vec<(String, i32)> {
    plan(profile, &target())
        .steps
        .iter()
        .filter_map(|planned| {
            let priority = match planned.step.kind.as_ref()? {
                step::Kind::Backup(backup) => backup.priority,
                step::Kind::Forget(forget) => forget.priority,
                step::Kind::Prune(prune) => prune.priority,
                step::Kind::Check(check) => check.priority,
                _ => return None,
            };
            Some((planned.step.step_id.clone(), priority))
        })
        .collect()
}

fn with_maintenance(priority: Priority) -> Profile {
    let mut profile = profile();
    profile.options.priority = priority;
    profile.retention = Some(Retention::default());
    profile.check = Some(CheckOptions::default());
    profile
}

#[test]
fn fr_ag_15_a_low_priority_profile_gives_every_restic_step_the_low_priority() {
    let low = ProcessPriority::Low as i32;
    assert_eq!(
        priorities(&with_maintenance(Priority::Low)),
        vec![
            ("backup".to_owned(), low),
            ("forget".to_owned(), low),
            ("prune".to_owned(), low),
            ("check".to_owned(), low),
        ]
    );
}

#[test]
fn ac_ag_14_a_normal_priority_profile_changes_no_step() {
    let unspecified = ProcessPriority::Unspecified as i32;
    let steps = priorities(&with_maintenance(Priority::Normal));
    assert_eq!(steps.len(), 4);
    assert!(steps.iter().all(|(_, priority)| *priority == unspecified));
}
