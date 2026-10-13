//! Process hardening for the control plane (ADR-0026, NFR-SEC-04).
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
