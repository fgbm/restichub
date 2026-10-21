//! Process hardening and process limits for the control plane (ADR-0026, NFR-SEC-04).
//!
//! On Linux the process clears `PR_SET_DUMPABLE`, so the kernel writes no core dump and a process
//! of the same user cannot read the memory through `/proc/<pid>/mem`. A core dump would hold the
//! unwrapped CA key bytes that `rcgen` and `ring` keep in the heap (T18). A service unit sets
//! `LimitCORE=0` as the second barrier.

/// Refuses a core dump of the process on Linux; does nothing on another platform.
pub fn disable_core_dumps() {
    #[cfg(target_os = "linux")]
    set_not_dumpable();
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn set_not_dumpable() {
    // SAFETY: `prctl` with `PR_SET_DUMPABLE` reads no pointer and writes no caller memory; it sets
    // one process attribute. The return value is ignored on purpose, because a failure must not
    // stop the start.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

/// Raises the soft open file limit to the hard limit and returns the soft limit in effect.
///
/// A container runtime can start the process with a soft limit of 1024 under a much larger hard
/// limit, and the agent listener needs a descriptor for each agent.
#[cfg(unix)]
pub fn raise_open_file_limit() -> std::io::Result<usize> {
    let mut limit = open_file_limit()?;
    if limit.rlim_cur < limit.rlim_max {
        limit.rlim_cur = limit.rlim_max;
        set_open_file_limit(&limit)?;
    }
    Ok(usize::try_from(limit.rlim_cur).unwrap_or(usize::MAX))
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn open_file_limit() -> std::io::Result<libc::rlimit> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes one `rlimit` into the valid, exclusively borrowed `limit`.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(limit)
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn set_open_file_limit(limit: &libc::rlimit) -> std::io::Result<()> {
    // SAFETY: `setrlimit` only reads the valid `rlimit` behind the reference.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn the_soft_open_file_limit_rises_to_the_hard_limit() {
        let before = open_file_limit().expect("the open file limit");
        let lowered = libc::rlimit {
            rlim_cur: before.rlim_max - 1,
            rlim_max: before.rlim_max,
        };
        set_open_file_limit(&lowered).expect("lowering the soft limit is always allowed");

        let raised = raise_open_file_limit().expect("the soft limit must rise");

        let after = open_file_limit().expect("the open file limit");
        assert_eq!(after.rlim_cur, before.rlim_max);
        assert_eq!(
            raised,
            usize::try_from(after.rlim_cur).unwrap_or(usize::MAX)
        );
    }
}
