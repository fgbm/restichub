//! The repair of the program directory before an install, and how the service starts after a
//! repair in place (FR-IN-07, T20).
//!
//! A repair in place leaves a handle that a process opened before it with its access, so until
//! the computer restarts that process can add a library that the service would load. The service
//! then starts only by hand, each install checks the tree before the start, and the first clean
//! run after a restart sets the automatic start again. A record under `HKLM` keeps that state.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::paths::ProgramRebuild;
use crate::service::ServiceManager;

#[cfg(windows)]
mod record;

/// What the run does with the program directory before the service starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Policy {
    Automatic,
    /// A repair in place waits for a restart: the tree is checked and the service starts by hand.
    AwaitRestart,
    /// The first run after the restart: the tree is checked once more and the record goes.
    Restarted,
}

/// Where the time of the last repair in place is kept.
pub(crate) trait Record {
    fn read(&self) -> Result<Option<i64>>;
    fn write(&self, repaired_at: i64) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

pub(crate) struct HostRecord;

impl Record for HostRecord {
    fn read(&self) -> Result<Option<i64>> {
        #[cfg(windows)]
        {
            record::read()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    fn write(&self, repaired_at: i64) -> Result<()> {
        #[cfg(windows)]
        {
            record::write(repaired_at)
        }
        #[cfg(not(windows))]
        {
            let _ = repaired_at;
            Ok(())
        }
    }

    fn clear(&self) -> Result<()> {
        #[cfg(windows)]
        {
            record::clear()
        }
        #[cfg(not(windows))]
        {
            Ok(())
        }
    }
}

fn boot_time() -> i64 {
    #[cfg(windows)]
    {
        record::boot_time()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Whether a repair in place waits for a clean run after a restart, so the service must not start
/// by itself at the boot (T20).
///
/// # Errors
///
/// Returns an error when the registry record cannot be read.
pub fn manual_start() -> Result<bool> {
    Ok(HostRecord.read()?.is_some())
}

/// Removes the record of a repair in place, for `uninstall-service`.
///
/// # Errors
///
/// Returns an error when the record exists and cannot be removed.
pub fn forget() -> Result<()> {
    HostRecord.clear()
}

#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) rebuilt: ProgramRebuild,
    pub(crate) policy: Policy,
}

impl Prepared {
    /// Refuses a tree that a handle from before a repair in place changed (T20).
    pub(crate) fn check_before_start(&self) -> Result<()> {
        if self.policy == Policy::Automatic {
            return Ok(());
        }
        crate::paths::check_program_files()
    }

    /// Runs right before the install: after a clean check after the restart, the record goes, so
    /// the install sets the automatic start again.
    pub(crate) fn start_service_from_tree(
        &self,
        record: &dyn Record,
        check: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        if self.policy != Policy::Automatic {
            check()?;
        }
        if self.policy == Policy::Restarted {
            record.clear()?;
        }
        Ok(())
    }

    pub(crate) fn warning(&self) -> Option<String> {
        match self.policy {
            Policy::AwaitRestart => Some(self.rebuilt.warning().unwrap_or_else(|| {
                format!(
                    "warning: the repair in place of {} waits for a restart of the computer; until then a process that kept it open can add a file there, so the service starts only by hand; restart the computer and run install-service again; it prints no repaired or moved lines and sets the automatic start again",
                    crate::paths::program_dir().display()
                )
            })),
            Policy::Restarted => Some(
                "the program directory passed the check after the restart, so the service starts automatically again".to_owned(),
            ),
            Policy::Automatic => None,
        }
    }
}

/// Makes an open program directory again or repairs it in place, then checks it and reads the
/// service state, so that the later copy meets no refusal (FR-IN-07, T20).
///
/// A warning about a repair in place comes at once, so a later refusal in the same run cannot
/// hide it.
pub(crate) fn prepare(config: &Path, manager: &dyn ServiceManager) -> Result<Prepared> {
    let running = crate::self_install::running_image()?;
    // A running service and its restic keep the directory in use. The command starts the service
    // at the end anyway. A stop in any state but the stopped one, as a stuck stop, is waited for.
    let rebuilt = crate::paths::rebuild_program_dir(
        config,
        running.path(),
        &mut || manager.stop(),
        &mut |line| println!("{line}"),
    )?;
    let prepared = settle(
        rebuilt,
        &HostRecord,
        boot_time(),
        time::OffsetDateTime::now_utc().unix_timestamp(),
        &mut |line| println!("{line}"),
    )?;
    crate::paths::ensure_program_dir(&crate::paths::restic_dir())?;
    crate::paths::ensure_program_dir(&crate::paths::program_dir())?;
    if prepared.rebuilt.replaced() {
        // The installed service starts from this path also when a later check refuses the identity.
        crate::self_install::copy_agent(&crate::self_install::Placement::host(), &running)?;
        println!(
            "the agent is installed in {}",
            crate::paths::program_dir().display()
        );
    }
    manager
        .state()
        .context("cannot read the state of the agent service")?;
    Ok(prepared)
}

/// A moved tree is new, so no old handle reaches it and an older record goes.
fn settle(
    rebuilt: ProgramRebuild,
    record: &dyn Record,
    boot: i64,
    now: i64,
    print: &mut dyn FnMut(&str),
) -> Result<Prepared> {
    let policy = match &rebuilt {
        ProgramRebuild::RepairedInPlace { .. } => {
            record.write(now)?;
            Policy::AwaitRestart
        }
        ProgramRebuild::Moved => {
            record.clear()?;
            Policy::Automatic
        }
        ProgramRebuild::Unchanged => match record.read()? {
            None => Policy::Automatic,
            Some(repaired_at) if boot > repaired_at => Policy::Restarted,
            Some(_) => Policy::AwaitRestart,
        },
    };
    let prepared = Prepared { rebuilt, policy };
    if policy == Policy::AwaitRestart
        && let Some(warning) = prepared.warning()
    {
        print(&warning);
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::cell::Cell;
    use std::path::PathBuf;

    use super::*;

    #[derive(Default)]
    struct Memory(Cell<Option<i64>>);

    impl Record for Memory {
        fn read(&self) -> Result<Option<i64>> {
            Ok(self.0.get())
        }

        fn write(&self, repaired_at: i64) -> Result<()> {
            self.0.set(Some(repaired_at));
            Ok(())
        }

        fn clear(&self) -> Result<()> {
            self.0.set(None);
            Ok(())
        }
    }

    fn in_place() -> ProgramRebuild {
        ProgramRebuild::RepairedInPlace {
            dir: PathBuf::from("ResticHub"),
            holders: "pwsh.exe (PID 7)".to_owned(),
        }
    }

    const BOOT: i64 = 1_000;
    const REPAIR: i64 = 2_000;
    const NEXT_BOOT: i64 = 3_000;

    fn settled(
        rebuilt: ProgramRebuild,
        record: &Memory,
        boot: i64,
        now: i64,
    ) -> (Prepared, Vec<String>) {
        let mut printed = Vec::new();
        let prepared = settle(rebuilt, record, boot, now, &mut |line| {
            printed.push(line.to_owned())
        })
        .expect("settle");
        (prepared, printed)
    }

    #[test]
    fn fr_in_07_a_repair_in_place_starts_the_service_by_hand_until_a_clean_run_after_a_restart() {
        let record = Memory::default();
        // The warning comes before the caller goes on, so a later refusal cannot hide it.
        let (repaired, printed) = settled(in_place(), &record, BOOT, REPAIR);
        assert_eq!(repaired.policy, Policy::AwaitRestart);
        assert_eq!(record.0.get(), Some(REPAIR));
        let [warning] = printed.as_slice() else {
            panic!("one warning: {printed:?}");
        };
        assert!(warning.contains("pwsh.exe (PID 7)"), "{warning}");
        assert!(warning.contains("starts only by hand"), "{warning}");
        assert!(
            warning.contains("close that process or remove it from the startup"),
            "{warning}"
        );

        let (same_boot, printed) = settled(ProgramRebuild::Unchanged, &record, BOOT, REPAIR + 60);
        assert_eq!(same_boot.policy, Policy::AwaitRestart);
        assert!(printed[0].contains("waits for a restart"), "{printed:?}");
        let mut checked = false;
        same_boot
            .start_service_from_tree(&record, || {
                checked = true;
                Ok(())
            })
            .expect("start");
        assert!(checked, "the tree is checked before the start");
        assert_eq!(
            record.0.get(),
            Some(REPAIR),
            "the record waits for the restart"
        );

        let (restarted, printed) = settled(
            ProgramRebuild::Unchanged,
            &record,
            NEXT_BOOT,
            NEXT_BOOT + 60,
        );
        assert_eq!(restarted.policy, Policy::Restarted);
        assert!(printed.is_empty(), "{printed:?}");
        restarted
            .start_service_from_tree(&record, || anyhow::bail!("a planted file"))
            .expect_err("a failed check keeps the record");
        assert_eq!(record.0.get(), Some(REPAIR));
        restarted
            .start_service_from_tree(&record, || Ok(()))
            .expect("the record goes before the install");
        assert_eq!(record.0.get(), None);

        let (later, printed) = settled(
            ProgramRebuild::Unchanged,
            &record,
            NEXT_BOOT,
            NEXT_BOOT + 120,
        );
        assert_eq!(later.policy, Policy::Automatic);
        assert!(printed.is_empty() && later.warning().is_none());
        later
            .start_service_from_tree(&record, || anyhow::bail!("no check without a repair"))
            .expect("no check");
    }

    #[test]
    fn fr_in_07_a_moved_tree_ends_a_waiting_repair_in_place() {
        let record = Memory::default();
        record.write(REPAIR).expect("write");
        let (moved, _) = settled(ProgramRebuild::Moved, &record, BOOT, REPAIR + 60);
        assert_eq!(moved.policy, Policy::Automatic);
        assert_eq!(record.0.get(), None);
    }

    #[test]
    fn fr_in_07_a_new_repair_in_place_after_a_restart_waits_again() {
        let record = Memory::default();
        record.write(REPAIR).expect("write");
        let (again, _) = settled(in_place(), &record, NEXT_BOOT, NEXT_BOOT + 60);
        assert_eq!(again.policy, Policy::AwaitRestart);
        assert_eq!(record.0.get(), Some(NEXT_BOOT + 60));
    }
}
