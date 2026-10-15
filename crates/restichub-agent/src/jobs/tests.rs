#![allow(clippy::expect_used)]

use ed25519_dalek::SigningKey;
use restichub_proto::agent::v1::{
    BackupStep, CheckStep, ForgetStep, HookStep, InitStep, Job, LsStep, OverwriteMode, PruneStep,
    RepositoryAccess, RestoreStep, SnapshotsFilter, SnapshotsStep, StatsStep, Step, TestAccessStep,
    UnlockAllStep, UnlockStep, step,
};
use restichub_proto::signing::{Expected, KeyRing, sign_job};

use super::{Progress, job_error, job_outcome, verify_and_validate};
use crate::journal::FileJournal;
use restichub_compiler::ValidatedJob;
use restichub_proto::agent::v1::{JobStatus, RejectReason, SignedJob};
use restichub_proto::signing::{Admission, admit};
use std::time::Duration;

/// The decision of the verification and the journal for one signed job.
enum Intake {
    Rejected(RejectReason),
    Completed(Box<restichub_proto::agent::v1::JobResult>),
    New(Box<ValidatedJob>),
}

/// Runs the same checks as `Jobs::handle` and admits the job with the expiry that it carries.
fn intake(
    signed: &SignedJob,
    ring: &KeyRing,
    expected: &Expected,
    journal: &mut FileJournal,
) -> Intake {
    match verify_and_validate(signed, ring, expected) {
        Ok(validated) => {
            let expires_at = validated.job().expires_at.unwrap_or(expected.now);
            match admit(validated.job(), journal, expires_at) {
                Admission::AlreadyCompleted(result) => Intake::Completed(Box::new(result)),
                Admission::New | Admission::InProgress => Intake::New(Box::new(validated)),
            }
        }
        Err(rejection) => Intake::Rejected(rejection.reason),
    }
}

const KEY_ID: &str = "job-key-1";

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

fn ring() -> KeyRing {
    let mut ring = KeyRing::new();
    ring.insert(KEY_ID, signing_key().verifying_key())
        .expect("insert");
    ring
}

fn at(seconds: i64) -> prost_types::Timestamp {
    prost_types::Timestamp { seconds, nanos: 0 }
}

fn expected() -> Expected {
    Expected {
        agent_id: "agent-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        now: at(1_000),
    }
}

fn test_access() -> Step {
    Step {
        step_id: "test-access".to_owned(),
        kind: Some(step::Kind::TestAccess(TestAccessStep {
            repository_id: "repo-1".to_owned(),
        })),
        ..Step::default()
    }
}

fn init() -> Step {
    Step {
        step_id: "init".to_owned(),
        kind: Some(step::Kind::Init(InitStep {
            repository_id: "repo-1".to_owned(),
        })),
        ..Step::default()
    }
}

fn job(agent: &str, tenant: &str, issued: i64, expires: i64, steps: Vec<Step>) -> Job {
    Job {
        job_id: "job-1".to_owned(),
        tenant_id: tenant.to_owned(),
        agent_id: agent.to_owned(),
        issued_at: Some(at(issued)),
        expires_at: Some(at(expires)),
        repositories: vec![RepositoryAccess {
            repository_id: "repo-1".to_owned(),
            location: "/srv/backup".to_owned(),
            password: "topsecret".to_owned(),
            ..RepositoryAccess::default()
        }],
        steps,
        ..Job::default()
    }
}

fn temp_journal(name: &str) -> FileJournal {
    let dir = std::env::temp_dir().join(format!(
        "restichub-agent-jobs-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    FileJournal::load(dir.join(crate::journal::JOURNAL_FILE)).expect("cannot load")
}

fn expected_reason(decision: &Intake) -> Option<RejectReason> {
    match decision {
        Intake::Rejected(reason) => Some(*reason),
        _ => None,
    }
}

#[test]
fn ac_pr_01_rejects_a_bad_signature() {
    let mut signed = sign_job(
        &job("agent-1", "tenant-1", 900, 2_000, vec![test_access()]),
        KEY_ID,
        &signing_key(),
    );
    signed.job[4] ^= 0xff;
    let mut journal = temp_journal("bad-signature");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    assert_eq!(
        expected_reason(&decision).as_ref(),
        Some(&RejectReason::BadSignature)
    );
}

#[test]
fn ac_pr_01_rejects_a_job_of_another_agent() {
    let signed = sign_job(
        &job("agent-2", "tenant-1", 900, 2_000, vec![test_access()]),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("wrong-agent");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    assert_eq!(
        expected_reason(&decision).as_ref(),
        Some(&RejectReason::WrongAgent)
    );
}

#[test]
fn ac_pr_01_rejects_a_job_of_another_tenant() {
    let signed = sign_job(
        &job("agent-1", "tenant-2", 900, 2_000, vec![test_access()]),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("wrong-tenant");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    assert_eq!(
        expected_reason(&decision).as_ref(),
        Some(&RejectReason::WrongTenant)
    );
}

#[test]
fn ac_pr_01_rejects_a_lifetime_over_24_hours() {
    let signed = sign_job(
        &job(
            "agent-1",
            "tenant-1",
            900,
            900 + 24 * 60 * 60 + 1,
            vec![test_access()],
        ),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("lifetime");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    assert_eq!(
        expected_reason(&decision).as_ref(),
        Some(&RejectReason::LifetimeTooLong)
    );
}

#[test]
fn ac_pr_01_rejects_a_future_issued_at() {
    let signed = sign_job(
        &job(
            "agent-1",
            "tenant-1",
            1_000 + 5 * 60 + 1,
            2_000,
            vec![test_access()],
        ),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("future");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    assert_eq!(
        expected_reason(&decision).as_ref(),
        Some(&RejectReason::NotYetValid)
    );
}

#[test]
fn ac_pr_01_rejects_an_expired_job() {
    let signed = sign_job(
        &job("agent-1", "tenant-1", 500, 800, vec![test_access()]),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("expired");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    assert_eq!(
        expected_reason(&decision).as_ref(),
        Some(&RejectReason::Expired)
    );
}

#[test]
fn ac_pr_01_a_replay_of_a_completed_job_returns_the_stored_result() {
    let signed = sign_job(
        &job("agent-1", "tenant-1", 900, 2_000, vec![test_access()]),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("replay");
    let first = intake(&signed, &ring(), &expected(), &mut journal);
    assert!(matches!(first, Intake::New(_)));
    // Completion stores the result in the journal.
    use restichub_proto::signing::JobJournal as _;
    journal.record_completed(
        "job-1",
        restichub_proto::agent::v1::JobResult {
            job_id: "job-1".to_owned(),
            ..Default::default()
        },
    );
    // After a restart the journal holds the result, so the agent does not run the job again.
    let replay = intake(&signed, &ring(), &expected(), &mut journal);
    match replay {
        Intake::Completed(result) => assert_eq!(result.job_id, "job-1"),
        _ => panic!("the replay must return the stored result"),
    }
}

#[test]
fn fr_pr_01_rejects_the_step_kinds_of_later_epics() {
    let steps = [step::Kind::Hook(HookStep {
        hook_id: "pg_dump".to_owned(),
        ..HookStep::default()
    })];
    for kind in steps {
        let step = Step {
            step_id: "later".to_owned(),
            kind: Some(kind),
            ..Step::default()
        };
        let signed = sign_job(
            &job("agent-1", "tenant-1", 900, 2_000, vec![step]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("unsupported");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::UnsupportedStep)
        );
    }
}

#[test]
fn fr_pr_02_accepts_the_step_kinds_of_the_agent_core() {
    let steps = [
        step::Kind::Backup(BackupStep {
            repository_id: "repo-1".to_owned(),
            paths: vec!["/srv".to_owned()],
            ..BackupStep::default()
        }),
        step::Kind::Check(CheckStep {
            repository_id: "repo-1".to_owned(),
            ..CheckStep::default()
        }),
        step::Kind::Snapshots(SnapshotsStep {
            repository_id: "repo-1".to_owned(),
            ..SnapshotsStep::default()
        }),
        step::Kind::Forget(ForgetStep {
            repository_id: "repo-1".to_owned(),
            keep_last: 1,
            filter: Some(SnapshotsFilter {
                tags: vec!["restichub-profile-a".to_owned()],
                ..SnapshotsFilter::default()
            }),
            ..ForgetStep::default()
        }),
        step::Kind::Ls(LsStep {
            repository_id: "repo-1".to_owned(),
            snapshot_id: "0123abcd".to_owned(),
            ..LsStep::default()
        }),
        step::Kind::Restore(RestoreStep {
            repository_id: "repo-1".to_owned(),
            snapshot_id: "0123abcd".to_owned(),
            target: "/restore".to_owned(),
            overwrite: OverwriteMode::Never as i32,
            ..RestoreStep::default()
        }),
        step::Kind::Prune(PruneStep {
            repository_id: "repo-1".to_owned(),
            ..PruneStep::default()
        }),
        step::Kind::Unlock(UnlockStep {
            repository_id: "repo-1".to_owned(),
        }),
        step::Kind::UnlockAll(UnlockAllStep {
            repository_id: "repo-1".to_owned(),
        }),
        step::Kind::Stats(StatsStep {
            repository_id: "repo-1".to_owned(),
            ..StatsStep::default()
        }),
    ];
    for kind in steps {
        let step = Step {
            step_id: "core".to_owned(),
            kind: Some(kind),
            ..Step::default()
        };
        let signed = sign_job(
            &job("agent-1", "tenant-1", 900, 2_000, vec![step]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("supported");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert!(matches!(decision, Intake::New(_)));
    }
}

fn progress(change: impl FnOnce(&mut Progress)) -> JobStatus {
    let mut progress = Progress::default();
    change(&mut progress);
    job_outcome(&progress)
}

#[test]
fn ac_rp_01_maps_the_init_chain_to_a_job_outcome() {
    // A repository that exists: test_access exit 0, init skipped.
    assert_eq!(progress(|p| p.had_init = true), JobStatus::Succeeded);
    // A missing repository with a successful init.
    assert_eq!(
        progress(|p| {
            p.probe_missing = true;
            p.had_init = true;
            p.init_ran = true;
            p.init_succeeded = true;
        }),
        JobStatus::Succeeded
    );
    // Init failure gives a failed job.
    assert_eq!(
        progress(|p| {
            p.probe_missing = true;
            p.had_init = true;
            p.init_ran = true;
        }),
        JobStatus::Failed
    );
    // A standalone test_access on a missing repository.
    assert_eq!(
        progress(|p| p.probe_missing = true),
        JobStatus::NotInitialized
    );
    // An init step without a matching test_access is an invalid sequence.
    assert_eq!(
        progress(|p| {
            p.had_init = true;
            p.init_invalid = true;
        }),
        JobStatus::Failed
    );
    // A wrong password on an existing repository (exit code 12).
    assert_eq!(
        progress(|p| p.stopped_code = Some(12)),
        JobStatus::ConfigError
    );
    // Any other failure (exit code 1, a timeout).
    assert_eq!(progress(|p| p.stopped_code = Some(1)), JobStatus::Failed);
    // The control plane cancelled the job before any step finished.
    assert_eq!(progress(|p| p.cancelled = true), JobStatus::Cancelled);
    assert_eq!(
        job_error(JobStatus::ConfigError).map(|error| error.code),
        Some("wrong_password".to_owned())
    );
    assert_eq!(
        job_error(JobStatus::NotInitialized).map(|error| error.code),
        Some("repository_missing".to_owned())
    );
}

#[test]
fn fr_ag_09_a_warning_and_a_lock_are_typed_at_the_job_level() {
    // A step with restic exit code 3 gives a job that succeeded with warnings.
    assert_eq!(
        progress(|p| p.warned = true),
        JobStatus::SucceededWithWarnings
    );
    assert_eq!(
        job_error(JobStatus::SucceededWithWarnings).map(|error| error.code),
        Some("succeeded_with_warnings".to_owned())
    );
    // A step with restic exit code 11 gives a locked repository, not a plain failure.
    assert_eq!(
        progress(|p| p.stopped_code = Some(11)),
        JobStatus::RepositoryLocked
    );
    assert_eq!(
        job_error(JobStatus::RepositoryLocked).map(|error| error.code),
        Some("repository_locked".to_owned())
    );
    // A later failure outweighs an earlier warning.
    assert_eq!(
        progress(|p| {
            p.warned = true;
            p.stopped_code = Some(1);
        }),
        JobStatus::Failed
    );
}

#[test]
fn fr_sc_06_the_agent_rejects_unlock_all_with_another_step() {
    let unlock_all = || Step {
        step_id: "unlock-all".to_owned(),
        kind: Some(step::Kind::UnlockAll(UnlockAllStep {
            repository_id: "repo-1".to_owned(),
        })),
        ..Step::default()
    };
    let signed = sign_job(
        &job(
            "agent-1",
            "tenant-1",
            900,
            2_000,
            vec![test_access(), unlock_all()],
        ),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("unlock-all-mixed");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    assert_eq!(
        expected_reason(&decision).as_ref(),
        Some(&RejectReason::Malformed)
    );
}

#[test]
fn fr_ag_05_a_step_without_a_timeout_uses_the_job_timeout_or_the_default() {
    // Step value wins over the job value.
    assert_eq!(super::step_timeout(600, 900), Duration::from_secs(600));
    // No step value: the job value stands in.
    assert_eq!(super::step_timeout(0, 900), Duration::from_secs(900));
    // Neither: the default.
    assert_eq!(super::step_timeout(0, 0), super::DEFAULT_STEP_TIMEOUT);
}

#[test]
fn a_repository_step_is_needed_for_the_chain() {
    // The init chain starts with a test_access step; the job of the control plane holds it.
    let steps = vec![test_access(), init()];
    let signed = sign_job(
        &job("agent-1", "tenant-1", 900, 2_000, steps),
        KEY_ID,
        &signing_key(),
    );
    let mut journal = temp_journal("chain");
    let decision = intake(&signed, &ring(), &expected(), &mut journal);
    match decision {
        Intake::New(validated) => assert_eq!(validated.job().steps.len(), 2),
        _ => panic!("the init chain must be accepted"),
    }
}

#[test]
fn fr_rp_08_the_agent_rejects_a_forget_without_a_tag_filter() {
    let filters = [
        None,
        Some(SnapshotsFilter::default()),
        Some(SnapshotsFilter {
            hosts: vec!["host-a".to_owned()],
            ..SnapshotsFilter::default()
        }),
    ];
    for filter in filters {
        let step = Step {
            step_id: "forget".to_owned(),
            kind: Some(step::Kind::Forget(ForgetStep {
                repository_id: "repo-1".to_owned(),
                keep_last: 1,
                filter,
                ..ForgetStep::default()
            })),
            ..Step::default()
        };
        let signed = sign_job(
            &job("agent-1", "tenant-1", 900, 2_000, vec![step]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("forget-no-tag");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert!(matches!(
            decision,
            Intake::Rejected(RejectReason::Malformed)
        ));
    }
}
