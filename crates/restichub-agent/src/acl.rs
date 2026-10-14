//! Extended POSIX access control lists (FR-AG-01, threat T20).
//!
//! A mode that a normal `stat` shows does not include an entry of an extended access control list,
//! so the install-location check reads the `system.posix_acl_access` attribute directly.

#![allow(unsafe_code)]

use std::path::Path;

#[cfg(target_os = "linux")]
use anyhow::Context as _;
use anyhow::Result;

/// Reports whether the path holds an extended POSIX access control list (threat T20).
///
/// # Errors
///
/// Returns an error when the attribute cannot be read for a reason other than its absence.
#[cfg(target_os = "linux")]
pub fn has_extended_acl(path: &Path) -> Result<bool> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    let name = CString::new("system.posix_acl_access").context("bad attribute name")?;
    let path = CString::new(path.as_os_str().as_bytes()).context("the path holds a NUL byte")?;
    // SAFETY: the path and the name are valid C strings; a null buffer asks for the size only.
    let size = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        let error = std::io::Error::last_os_error();
        // ENODATA means that the attribute is absent, so the mode is the whole access control.
        if error.raw_os_error() == Some(libc::ENODATA) {
            return Ok(false);
        }
        return Err(anyhow::Error::from(error).context("cannot read the access control list"));
    }
    Ok(size > 0)
}

/// Reports whether the path holds an extended POSIX access control list (threat T20).
#[cfg(not(target_os = "linux"))]
pub fn has_extended_acl(_path: &Path) -> Result<bool> {
    Ok(false)
}
