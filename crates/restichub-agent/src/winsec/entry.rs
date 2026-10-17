//! One file or directory that the repair checks and changes through a single handle (threat T20).
//!
//! The handle opens the entry itself and never the target of a reparse point, so a link that takes
//! the place of the path after the check cannot redirect the change.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SetKernelObjectSecurity,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
};

use super::{
    AGENT_RIGHTS, DIR_SDDL, FILE_SDDL, OwnedDescriptor, descriptor, first_foreign_grant,
    is_trusted_sid, wide_path,
};

/// What the owner and the DACL of an entry allow to another account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exposure {
    Clean,
    /// SYSTEM or Administrators own it, but the DACL grants another account one of the rights.
    ForeignGrant,
    ForeignOwner,
}

/// The rights that another account must not hold on an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rights {
    Change,
    /// A read counts too, because the entry holds the agent identity.
    ReadOrChange,
}

impl Rights {
    fn mask(self) -> u32 {
        match self {
            Self::Change => AGENT_RIGHTS,
            Self::ReadOrChange => AGENT_RIGHTS | FILE_READ_DATA,
        }
    }
}

pub(crate) struct Entry {
    handle: HANDLE,
    info: BY_HANDLE_FILE_INFORMATION,
}

impl Drop for Entry {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is closed once.
        unsafe { CloseHandle(self.handle) };
    }
}

impl Entry {
    /// Opens the entry at `path`, or returns `None` when the path does not exist.
    pub(crate) fn open(path: &Path) -> Result<Option<Self>> {
        Self::open_with(path, READ_CONTROL | WRITE_DAC)
    }

    /// Opens the entry for [`Entry::own_and_protect`] too.
    pub(crate) fn open_to_own(path: &Path) -> Result<Option<Self>> {
        Self::open_with(path, READ_CONTROL | WRITE_DAC | WRITE_OWNER)
    }

    /// Opens the entry for [`Entry::exposure`] only, so a process without elevation can check it.
    pub(crate) fn inspect(path: &Path) -> Result<Option<Self>> {
        Self::open_with(path, READ_CONTROL)
    }

    fn open_with(path: &Path, access: u32) -> Result<Option<Self>> {
        let wide = wide_path(path);
        // SAFETY: the path is a null-terminated UTF-16 string. The access asks for no data, so the
        // share mode of another handle cannot refuse it.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error).with_context(|| format!("cannot open {}", path.display()));
        }
        let mut entry = Self {
            handle,
            info: BY_HANDLE_FILE_INFORMATION::default(),
        };
        // SAFETY: the handle is open and the out parameter is a field of the entry.
        let ok = unsafe { GetFileInformationByHandle(entry.handle, &mut entry.info) };
        if ok == 0 {
            bail!(
                "cannot read {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            );
        }
        Ok(Some(entry))
    }

    pub(crate) fn is_dir(&self) -> bool {
        self.info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    pub(crate) fn is_reparse_point(&self) -> bool {
        self.info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    /// The number of names of the file. Each name can carry the content of another path.
    pub(crate) fn links(&self) -> u32 {
        self.info.nNumberOfLinks
    }

    pub(crate) fn exposure(&self, rights: Rights) -> Result<Exposure> {
        let mut owner: PSID = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut raw: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: the handle is open with READ_CONTROL and the out parameters point to locals.
        let status = unsafe {
            GetSecurityInfo(
                self.handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut raw,
            )
        };
        if status != ERROR_SUCCESS {
            bail!(
                "cannot read the access control: {}",
                std::io::Error::from_raw_os_error(status as i32)
            );
        }
        let _descriptor = OwnedDescriptor(raw);
        if owner.is_null() || !is_trusted_sid(owner) {
            return Ok(Exposure::ForeignOwner);
        }
        Ok(if first_foreign_grant(dacl, rights.mask())?.is_some() {
            Exposure::ForeignGrant
        } else {
            Exposure::Clean
        })
    }

    /// Sets the protected DACL of SYSTEM and Administrators on this entry only.
    pub(crate) fn protect(&self) -> Result<()> {
        self.set(DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION)
    }

    /// Sets Administrators as the owner too. Needs an entry from [`Entry::open_to_own`].
    pub(crate) fn own_and_protect(&self) -> Result<()> {
        self.set(
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
        )
    }

    fn set(&self, info: u32) -> Result<()> {
        let descriptor = descriptor(if self.is_dir() { DIR_SDDL } else { FILE_SDDL })?;
        // `SetSecurityInfo` would flow the DACL into the children, past a junction among them, so
        // the low-level call sets this object alone and the caller walks the children itself.
        // SAFETY: the handle is open with the rights that `info` needs and the descriptor stays
        // alive for the call.
        let ok = unsafe { SetKernelObjectSecurity(self.handle, info, descriptor.0) };
        if ok == 0 {
            bail!(
                "cannot set the access control: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }
}
