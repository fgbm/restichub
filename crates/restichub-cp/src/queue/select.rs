//! The selection of the jobs that start now: the repository barriers and the limits (FR-SC-04,
//! FR-SC-08, ADR-0010).
//!
//! The function is pure. The dispatcher passes the waiting jobs in the order of creation and the
//! jobs that already hold a slot; the function returns the jobs that may start. The order is the
//! fairness rule: a waiting exclusive job (a prune, for example) blocks the later jobs of its
//! repository, so a stream of new backups cannot starve it. A browse job passes a waiting
//! `unlock` only: it answers a person who waits (FR-RS-02), and an `unlock` can wait a long time
//! for a running backup. A waiting prune, forget, check or `unlock_all` holds browse jobs back.

use std::collections::{HashMap, HashSet};

use restichub_db::jobs::Job;
use restichub_db::queue::InFlight;
use restichub_db::{AgentId, JobKind, JobStatus, RepositoryId};

/// Caps on concurrent jobs in one tenant (FR-SC-08): per agent, per repository (the shared kinds
/// only) and in total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub agent: usize,
    pub repository: usize,
    pub global: usize,
}

#[derive(Default)]
struct RepositoryLoad {
    shared: usize,
    exclusive: bool,
}

/// The repositories with a waiting exclusive job, and which of them hold browse jobs back.
#[derive(Default)]
struct Barriers {
    all: HashSet<RepositoryId>,
    against_browse: HashSet<RepositoryId>,
}

impl Barriers {
    fn insert(&mut self, repository: RepositoryId, kind: JobKind) {
        self.all.insert(repository);
        if kind != JobKind::Unlock {
            self.against_browse.insert(repository);
        }
    }

    /// A browse job only reads, so a waiting `unlock` does not hold it back. A running exclusive
    /// job still does (FR-RS-02).
    fn holds_back(&self, repository: RepositoryId, kind: JobKind) -> bool {
        if kind.is_interactive() {
            self.against_browse.contains(&repository)
        } else {
            self.all.contains(&repository)
        }
    }
}

/// Returns the jobs that may be sent now, in the order of the input.
///
/// A job with the status `sent` is a redelivery after an expired lease. It already holds its slot,
/// so it bypasses the barriers and the limits.
pub fn select<'a>(waiting: &'a [Job], in_flight: &[InFlight], limits: &Limits) -> Vec<&'a Job> {
    let mut per_agent: HashMap<AgentId, usize> = HashMap::new();
    let mut per_repository: HashMap<RepositoryId, RepositoryLoad> = HashMap::new();
    for flight in in_flight {
        // A browse job answers a person who waits, so it holds no slot of its agent (FR-RS-02).
        if !flight.kind.is_interactive() {
            *per_agent.entry(flight.agent_id).or_default() += 1;
        }
        if let Some(repository) = flight.repository_id {
            let load = per_repository.entry(repository).or_default();
            if flight.kind.is_exclusive() {
                load.exclusive = true;
            } else {
                load.shared += 1;
            }
        }
    }
    let mut total = in_flight.len();

    // A repository with an exclusive job that waits blocks every later job. A repository with a
    // skipped job holds back a later exclusive job, so the order of the queue holds.
    let mut blocked = Barriers::default();
    let mut skipped: HashSet<RepositoryId> = HashSet::new();
    let mut selected = Vec::new();

    for job in waiting {
        if job.status == JobStatus::Sent {
            selected.push(job);
            continue;
        }
        let repository = job.repository_id;
        let exclusive = job.kind.is_exclusive();
        if total >= limits.global {
            if let Some(repository) = repository {
                skipped.insert(repository);
                if exclusive {
                    blocked.insert(repository, job.kind);
                }
            }
            continue;
        }
        if let Some(repository) = repository {
            if blocked.holds_back(repository, job.kind) {
                if exclusive {
                    blocked.insert(repository, job.kind);
                }
                continue;
            }
            let load = per_repository.entry(repository).or_default();
            let repository_ready = if exclusive {
                !load.exclusive && load.shared == 0 && !skipped.contains(&repository)
            } else if job.kind.is_interactive() {
                !load.exclusive
            } else {
                !load.exclusive && load.shared < limits.repository
            };
            if !repository_ready {
                skipped.insert(repository);
                if exclusive {
                    blocked.insert(repository, job.kind);
                }
                continue;
            }
        }
        let agent_count = per_agent.entry(job.agent_id).or_default();
        if *agent_count >= limits.agent && !job.kind.is_interactive() {
            if let Some(repository) = repository {
                skipped.insert(repository);
                if exclusive {
                    blocked.insert(repository, job.kind);
                }
            }
            continue;
        }

        if !job.kind.is_interactive() {
            *agent_count += 1;
        }
        total += 1;
        if let Some(repository) = repository {
            let load = per_repository.entry(repository).or_default();
            if exclusive {
                load.exclusive = true;
            } else {
                load.shared += 1;
            }
        }
        selected.push(job);
    }
    selected
}

#[cfg(test)]
mod tests {
    use restichub_db::{JobId, JobKind};
    use time::OffsetDateTime;

    use super::*;

    fn job(kind: JobKind, agent: AgentId, repository: RepositoryId, status: JobStatus) -> Job {
        let now = OffsetDateTime::UNIX_EPOCH;
        Job {
            id: JobId::new(),
            kind,
            agent_id: agent,
            repository_id: Some(repository),
            plan_id: None,
            fire_time: None,
            status,
            result_code: None,
            result_message: None,
            result_params: None,
            location_fingerprint: None,
            expires_at: None,
            sent_at: None,
            started_at: None,
            finished_at: None,
            created_at: now,
            updated_at: now,
            created_by: None,
            run_at: now,
            lease_until: None,
            attempt: 0,
            deliveries: 0,
            stale_lock_since: None,
            cancel_requested: false,
            lock_reported: false,
            empty_forget_reported: false,
            retry_of: None,
            timeout_seconds: None,
            stall_timeout_seconds: None,
        }
    }

    fn running(kind: JobKind, agent: AgentId, repository: RepositoryId) -> InFlight {
        InFlight {
            agent_id: agent,
            repository_id: Some(repository),
            kind,
        }
    }

    const WIDE: Limits = Limits {
        agent: 100,
        repository: 100,
        global: 100,
    };

    #[test]
    fn ac_sc_03_a_prune_waits_for_every_running_backup() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights: Vec<_> = (0..3)
            .map(|_| running(JobKind::Backup, agent, repository))
            .collect();
        let prune = job(JobKind::Prune, agent, repository, JobStatus::Queued);
        assert!(select(std::slice::from_ref(&prune), &flights, &WIDE).is_empty());
        let idle = select(std::slice::from_ref(&prune), &[], &WIDE);
        assert_eq!(idle.len(), 1);
    }

    #[test]
    fn ac_sc_03_a_backup_queued_after_a_prune_waits_for_it() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights = vec![running(JobKind::Backup, agent, repository)];
        let waiting = vec![
            job(JobKind::Prune, agent, repository, JobStatus::Queued),
            job(JobKind::Backup, agent, repository, JobStatus::Queued),
        ];
        assert!(select(&waiting, &flights, &WIDE).is_empty());
        let after_prune = vec![running(JobKind::Prune, agent, repository)];
        let backup = vec![job(JobKind::Backup, agent, repository, JobStatus::Queued)];
        assert!(select(&backup, &after_prune, &WIDE).is_empty());
    }

    /// FR-SC-04: a read operation is no barrier, so a backup starts next to a running test access.
    #[test]
    fn a_backup_starts_next_to_a_running_test_access() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights = vec![running(JobKind::TestAccess, agent, repository)];
        let backup = vec![job(JobKind::Backup, agent, repository, JobStatus::Queued)];
        assert_eq!(select(&backup, &flights, &WIDE).len(), 1);
    }

    #[test]
    fn ac_sc_03_a_backup_queued_before_a_prune_starts_first() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let waiting = vec![
            job(JobKind::Backup, agent, repository, JobStatus::Queued),
            job(JobKind::Prune, agent, repository, JobStatus::Queued),
        ];
        let chosen = select(&waiting, &[], &WIDE);
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].kind, JobKind::Backup);
    }

    #[test]
    fn ac_sc_06_a_browse_job_passes_a_waiting_unlock_and_a_backup_does_not() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights = vec![running(JobKind::Backup, agent, repository)];
        let waiting = vec![
            job(JobKind::Unlock, agent, repository, JobStatus::Queued),
            job(JobKind::Backup, agent, repository, JobStatus::Queued),
            job(JobKind::Snapshots, agent, repository, JobStatus::Queued),
            job(JobKind::Ls, agent, repository, JobStatus::Queued),
            job(JobKind::Restore, agent, repository, JobStatus::Queued),
        ];
        let kinds: Vec<_> = select(&waiting, &flights, &WIDE)
            .iter()
            .map(|job| job.kind)
            .collect();
        assert_eq!(kinds, vec![JobKind::Snapshots, JobKind::Ls]);
    }

    #[test]
    fn ac_sc_03_a_browse_job_waits_behind_a_waiting_prune() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights = vec![running(JobKind::Backup, agent, repository)];
        let waiting = vec![
            job(JobKind::Prune, agent, repository, JobStatus::Queued),
            job(JobKind::Snapshots, agent, repository, JobStatus::Queued),
        ];
        assert!(select(&waiting, &flights, &WIDE).is_empty());
    }

    #[test]
    fn ac_sc_03_a_browse_job_waits_behind_a_prune_that_waits_behind_an_unlock() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights = vec![running(JobKind::Backup, agent, repository)];
        let waiting = vec![
            job(JobKind::Unlock, agent, repository, JobStatus::Queued),
            job(JobKind::Prune, agent, repository, JobStatus::Queued),
            job(JobKind::Snapshots, agent, repository, JobStatus::Queued),
        ];
        assert!(select(&waiting, &flights, &WIDE).is_empty());
    }

    #[test]
    fn ac_sc_06_a_browse_job_waits_for_a_running_unlock() {
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights = vec![running(JobKind::Unlock, agent, repository)];
        let waiting = vec![job(
            JobKind::Snapshots,
            agent,
            repository,
            JobStatus::Queued,
        )];
        assert!(select(&waiting, &flights, &WIDE).is_empty());
    }

    #[test]
    fn ac_sc_08_no_limit_is_exceeded() {
        let limits = Limits {
            agent: 1,
            repository: 2,
            global: 100,
        };
        let agents: Vec<_> = (0..2).map(|_| AgentId::new()).collect();
        let repositories: Vec<_> = (0..2).map(|_| RepositoryId::new()).collect();
        let waiting: Vec<_> = (0..10)
            .map(|index| {
                job(
                    JobKind::Backup,
                    agents[index % 2],
                    repositories[(index / 2) % 2],
                    JobStatus::Queued,
                )
            })
            .collect();
        let chosen = select(&waiting, &[], &limits);
        let mut per_agent: HashMap<AgentId, usize> = HashMap::new();
        let mut per_repository: HashMap<RepositoryId, usize> = HashMap::new();
        for job in &chosen {
            *per_agent.entry(job.agent_id).or_default() += 1;
            if let Some(repository) = job.repository_id {
                *per_repository.entry(repository).or_default() += 1;
            }
        }
        assert!(per_agent.values().all(|count| *count <= 1));
        assert!(per_repository.values().all(|count| *count <= 2));
        assert_eq!(chosen.len(), 2);
    }

    #[test]
    fn ac_sc_08_the_global_limit_holds() {
        let limits = Limits {
            agent: 100,
            repository: 100,
            global: 3,
        };
        let waiting: Vec<_> = (0..10)
            .map(|_| {
                job(
                    JobKind::Backup,
                    AgentId::new(),
                    RepositoryId::new(),
                    JobStatus::Queued,
                )
            })
            .collect();
        let flights = vec![running(
            JobKind::Backup,
            AgentId::new(),
            RepositoryId::new(),
        )];
        assert_eq!(select(&waiting, &flights, &limits).len(), 2);
    }

    #[test]
    fn ac_pr_04_a_redelivery_bypasses_the_limits() {
        let limits = Limits {
            agent: 1,
            repository: 1,
            global: 1,
        };
        let (agent, repository) = (AgentId::new(), RepositoryId::new());
        let flights = vec![running(JobKind::Backup, agent, repository)];
        let waiting = vec![job(JobKind::Backup, agent, repository, JobStatus::Sent)];
        assert_eq!(select(&waiting, &flights, &limits).len(), 1);
    }
}
