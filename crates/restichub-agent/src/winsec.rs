//! The protected directory and the file access control on Windows (FR-AG-04, threat T20).
//!
//! The identity must stay on SYSTEM and on Administrators. The module builds an explicit security
//! descriptor and applies it with the Windows API, so no inherited access control entry (a normal
//! `%ProgramData%` grants a normal user a read) reaches the key. A directory that already exists
//! with another owner, or with a write access for another account, is refused before any file is
//! written.
//!
//! The check walks the DACL with `GetAclInformation` and `GetAce` and reads the numeric access
//! mask, so it never parses the text form of the security descriptor. A generic right is mapped
//! with the file mapping before the test. An allow entry of a type this module does not parse fails
//! closed.

#![allow(unsafe_code)]

use std::io::Write as _;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::MetadataExt as _;
use std::os::windows::io::FromRawHandle as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
    LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    CheckTokenMembership, DACL_SECURITY_INFORMATION, EqualSid, GENERIC_MAPPING, GetAce,
    GetAclInformation, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetTokenInformation,
    INHERIT_ONLY_ACE, MapGenericMask, OWNER_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateDirectoryW, CreateFileW, DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
    FILE_ALL_ACCESS, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA,
    FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::SystemServices::{
    ACCESS_ALLOWED_ACE_TYPE, ACCESS_ALLOWED_CALLBACK_ACE_TYPE,
    ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE, ACCESS_ALLOWED_OBJECT_ACE_TYPE,
    ACCESS_DENIED_ACE_TYPE, ACCESS_DENIED_CALLBACK_ACE_TYPE,
    ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE, ACCESS_DENIED_OBJECT_ACE_TYPE, SYSTEM_ALARM_ACE_TYPE,
    SYSTEM_ALARM_OBJECT_ACE_TYPE, SYSTEM_AUDIT_ACE_TYPE, SYSTEM_AUDIT_OBJECT_ACE_TYPE,
    SYSTEM_MANDATORY_LABEL_ACE_TYPE, SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE,
    SYSTEM_SCOPED_POLICY_ID_ACE_TYPE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

mod entry;
mod holders;
mod image;

pub(crate) use entry::{Entry, Exposure, Rights};
pub(crate) use holders::holders;
pub(crate) use image::open_running_image;

/// A protected DACL: full control for SYSTEM and Administrators only, inherited by the children,
/// with Administrators as the owner.
const DIR_SDDL: &str = "O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// A protected DACL for one file: full control for SYSTEM and Administrators only.
const FILE_SDDL: &str = "O:BAG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)";

/// The security identifiers that may own or change the agent: SYSTEM, BUILTIN\Administrators and
/// the TrustedInstaller service account that the Windows installer uses.
const TRUSTED_SIDS: [&str; 3] = [
    "S-1-5-18",
    "S-1-5-32-544",
    // The TrustedInstaller service account of the Windows installer.
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
];

/// The rights that another account must not hold on the agent, on restic or on the identity
/// directory: a change of the data, of the attributes, of the access control, or a delete.
const AGENT_RIGHTS: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | FILE_DELETE_CHILD
    | DELETE
    | WRITE_DAC
    | WRITE_OWNER;

/// The rights that another account must not hold on a parent directory of the agent: a delete, a
/// delete of a child, or a change of the access control. A normal `C:\` grants Authenticated Users
/// the right to add a file, which stays allowed.
const PARENT_RIGHTS: u32 = FILE_DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER;

/// The rights that another account must not hold on a directory that the agent searches for a
/// program or runs one from: the rights of a parent, and the right to add a file or a
/// subdirectory, which plants a program or a library next to a trusted one.
const PROGRAM_DIRECTORY_RIGHTS: u32 = PARENT_RIGHTS | FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY;

/// Owns a security descriptor and frees it on drop.
struct OwnedDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for OwnedDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0 as HLOCAL) };
        }
    }
}

/// Owns a security identifier and frees it on drop.
struct OwnedSid(PSID);

impl Drop for OwnedSid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0 as HLOCAL) };
        }
    }
}

/// Makes `dir` and every missing ancestor with a protected DACL for SYSTEM and Administrators
/// (FR-AG-04).
///
/// An existing level at or below `root`, and `dir` itself, must be SYSTEM and Administrators only
/// and is left as it is, so a repeated call does not walk the DACL over the children. A reparse
/// point at such a level is refused. An existing ancestor above `root` is not read or changed.
///
/// # Errors
///
/// Returns an error when a level cannot be created, when an existing level is a reparse point, has
/// another owner or a write access for another account, or when the access control cannot be set.
pub fn create_protected_tree(dir: &Path, root: &Path) -> Result<()> {
    protect_tree(dir, root, &crate::paths::agent_root(), &mut |level| {
        create_dir(level)?;
        apply_descriptor(level, DIR_SDDL)
    })
}

/// A refused level below `repaired` names the command that repairs it.
fn protect_tree(
    dir: &Path,
    root: &Path,
    repaired: &Path,
    create: &mut dyn FnMut(&Path) -> Result<()>,
) -> Result<()> {
    let mut levels = Vec::new();
    let mut level = Some(dir);
    while let Some(current) = level {
        if current != dir
            && !current.starts_with(root)
            && std::fs::symlink_metadata(current).is_ok()
        {
            break;
        }
        levels.push(current);
        level = current
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
    }
    // The top level comes first, so an unsafe existing level is refused before anything is created
    // below it.
    for level in levels.into_iter().rev() {
        match std::fs::symlink_metadata(level) {
            Ok(metadata) => {
                // A junction or a symbolic link would make the DACL check read the target.
                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    bail!(
                        "{} is a reparse point; an administrator removes the link, then runs the command again",
                        level.display()
                    );
                }
                if level.starts_with(repaired) {
                    check_with_advice(level, AGENT_RIGHTS, REPAIR_ADVICE)
                } else {
                    check_admin_only(level)
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => create(level),
            Err(error) => Err(error).with_context(|| format!("cannot read {}", level.display())),
        }
        .with_context(|| format!("cannot protect the directory {}", level.display()))?;
    }
    Ok(())
}

/// Tells whether the process token is elevated and a member of Administrators (FR-IN-07).
pub fn is_elevated_administrator() -> Result<bool> {
    let size = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>())
        .context("the token elevation is too large")?;
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo handle of the current process is always valid and the out parameter is a
    // local.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 {
        bail!(
            "cannot open the process token: {}",
            std::io::Error::last_os_error()
        );
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    // SAFETY: the token is open and the buffer is a local of the size that is passed.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            std::ptr::addr_of_mut!(elevation).cast(),
            size,
            &mut returned,
        )
    };
    let read_error = std::io::Error::last_os_error();
    // SAFETY: the token was opened above and is closed once.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        bail!("cannot read the token elevation: {read_error}");
    }
    if elevation.TokenIsElevated == 0 {
        return Ok(false);
    }
    let Some(administrators) = sid_from_string("S-1-5-32-544") else {
        bail!("cannot build the Administrators identifier");
    };
    let mut member = 0;
    // SAFETY: a null token means the token of the calling thread or process; the SID is valid.
    let ok = unsafe { CheckTokenMembership(std::ptr::null_mut(), administrators.0, &mut member) };
    if ok == 0 {
        bail!(
            "cannot check the Administrators membership: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(member != 0)
}

/// The `MachineGuid` of this computer from the 64-bit view of the registry (FR-ID-01).
pub fn machine_guid() -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
    };
    let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "MachineGuid\0".encode_utf16().collect();
    let mut buffer = [0u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: both names end with a NUL, and `size` is the byte length of `buffer`.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units = (size as usize / 2).saturating_sub(1).min(buffer.len());
    String::from_utf16(&buffer[..units]).ok()
}

/// Makes `dir` with a protected DACL, or refuses an existing `dir` (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the directory cannot be created, when it exists with another owner or with
/// a write access for another account, or when the access control cannot be set.
pub fn create_protected_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        check_admin_only(dir)?;
    } else {
        create_dir(dir)?;
    }
    apply_descriptor(dir, DIR_SDDL)
}

/// Writes a file that only SYSTEM and Administrators can read (FR-AG-04).
///
/// An old file is removed first, so a symbolic link or a hard link cannot redirect the write.
///
/// # Errors
///
/// Returns an error when the file cannot be created or its access control cannot be set.
pub fn create_protected_file(path: &Path, contents: &[u8]) -> Result<()> {
    let _ = std::fs::remove_file(path);
    let mut file = create_new_protected(path, GENERIC_WRITE, 0)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("cannot flush {}", path.display()))
}

/// Opens the file, or creates it empty with the descriptor of [`create_protected_file`].
///
/// An existing file is never replaced: another process may hold a lock on it.
///
/// # Errors
///
/// Returns an error when the file cannot be opened or created, or its access control cannot be
/// set.
pub(crate) fn open_or_create_protected(path: &Path) -> Result<std::fs::File> {
    match create_new_protected(
        path,
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    ) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .with_context(|| format!("cannot open {}", path.display()))
        }
        Err(error) => Err(error).with_context(|| format!("cannot create {}", path.display())),
    }
}

/// Creates a new file with the protected descriptor of a file. The descriptor applies at the
/// creation, so no handle ever opens the file with the access of the parent.
fn create_new_protected(path: &Path, access: u32, share: u32) -> std::io::Result<std::fs::File> {
    let descriptor = descriptor(FILE_SDDL).map_err(std::io::Error::other)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let wide = wide_path(path);
    // SAFETY: the path is a null-terminated UTF-16 string and the attributes stay alive for the call.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            share,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the handle is open and the file takes the only ownership of it.
    Ok(unsafe { std::fs::File::from_raw_handle(handle) })
}

/// Refuses a path that another account owns or can change (FR-AG-04).
///
/// # Errors
///
/// Returns an error when another account owns the path or holds a write access on it.
pub fn check_admin_only(path: &Path) -> Result<()> {
    check(path, AGENT_RIGHTS)
}

/// Refuses a parent directory that another account can delete or re-permission (threat T20).
///
/// # Errors
///
/// Returns an error when another account owns the directory or can delete it or change its access.
pub fn check_parent(path: &Path) -> Result<()> {
    check(path, PARENT_RIGHTS)
}

/// Refuses a directory in which another account can add a file or a subdirectory, or that it can
/// delete or re-permission (threat T20).
///
/// # Errors
///
/// Returns an error when another account owns the directory or holds one of these rights on it.
pub fn check_program_directory(path: &Path) -> Result<()> {
    check(path, PROGRAM_DIRECTORY_RIGHTS)
}

/// Only the directories of the agent below `%ProgramData%\ResticHub` have a repair.
const REPAIR_ADVICE: &str = "run `restichub-agent install-service` as an administrator (`restichub-agent register` on a host without an identity), which repairs the agent directories";

const REMOVE_ENTRY_ADVICE: &str = "an administrator removes the entry of that account from the access control list of the path, then runs the command again";

fn check(path: &Path, rights: u32) -> Result<()> {
    check_with_advice(path, rights, REMOVE_ENTRY_ADVICE)
}

fn check_with_advice(path: &Path, rights: u32, advice: &str) -> Result<()> {
    let security = open_security(path)?;
    check_owner(path, security.owner)?;
    if let Some(mask) = first_foreign_grant(security.dacl, rights)? {
        bail!(
            "{} grants the access mask {mask:#010x} to an account that is not SYSTEM or Administrators; {advice}",
            path.display()
        );
    }
    Ok(())
}

/// The owner and the DACL of one object, with the descriptor that owns them.
struct Security {
    owner: PSID,
    dacl: *mut ACL,
    _descriptor: OwnedDescriptor,
}

fn open_security(path: &Path) -> Result<Security> {
    let wide = wide_path(path);
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    // SAFETY: the path is a null-terminated UTF-16 string and the out parameters point to locals.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            info,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        bail!(
            "cannot read the access control of {}: {} (error {status})",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    if descriptor.is_null() {
        bail!("the access control of {} is empty", path.display());
    }
    Ok(Security {
        owner,
        dacl,
        _descriptor: OwnedDescriptor(descriptor),
    })
}

fn check_owner(path: &Path, owner: PSID) -> Result<()> {
    if owner.is_null() || !is_trusted_sid(owner) {
        bail!("{}", foreign_owner_advice(path));
    }
    Ok(())
}

/// The advice for a path that another account owns. The agent never repairs it: the owner can
/// change the access control and may have put files there. Deleting a directory would also delete
/// the agent identity, so the text asks for a check of the files, then a reset.
pub(crate) fn foreign_owner_advice(path: &Path) -> String {
    let shown = path.display();
    if path.is_dir() {
        format!(
            "{shown} is owned by an account that is not SYSTEM or Administrators, so that account may have put files there; check the files in it first, then take the ownership and reset the access control in an elevated PowerShell, and run the command again:\n  icacls \"{shown}\" /setowner *S-1-5-32-544\n  $acl = New-Object System.Security.AccessControl.DirectorySecurity; $acl.SetSecurityDescriptorSddlForm('{DIR_SDDL}'); Set-Acl -LiteralPath \"{shown}\" -AclObject $acl"
        )
    } else {
        format!(
            "{shown} is owned by an account that is not SYSTEM or Administrators; an administrator checks the file, removes it and runs the command again; when it is a file of the agent identity, the removal loses the identity, so then {}",
            crate::winrepair::REENROLL_ADVICE
        )
    }
}

/// Returns the mask that a non-admin holds, or `None` when no entry grants it.
///
/// The mask is the granted part of `rights`, so a full control shows as `0x000d0156` against the
/// agent rights: every right of [`AGENT_RIGHTS`].
///
/// A missing DACL grants everyone full access, so it gives [`u32::MAX`]. An allow entry of a type
/// this module does not parse also gives [`u32::MAX`], because it may hold any right.
fn first_foreign_grant(dacl: *mut ACL, rights: u32) -> Result<Option<u32>> {
    if dacl.is_null() {
        return Ok(Some(u32::MAX));
    }
    let mut size = ACL_SIZE_INFORMATION::default();
    // SAFETY: the DACL is a valid ACL and the out parameter is a local.
    let ok = unsafe {
        GetAclInformation(
            dacl,
            std::ptr::addr_of_mut!(size).cast(),
            u32::try_from(std::mem::size_of::<ACL_SIZE_INFORMATION>())
                .context("the ACL size information is too large")?,
            AclSizeInformation,
        )
    };
    if ok == 0 {
        bail!(
            "cannot read an access control list: {}",
            std::io::Error::last_os_error()
        );
    }
    let mapping = file_generic_mapping();
    for index in 0..size.AceCount {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: the DACL is valid and the index is inside the list.
        let ok = unsafe { GetAce(dacl, index, &mut ace) };
        if ok == 0 || ace.is_null() {
            bail!(
                "cannot read an access control entry: {}",
                std::io::Error::last_os_error()
            );
        }
        // SAFETY: every entry starts with an ACE_HEADER.
        let header = unsafe { *(ace as *const ACE_HEADER) };
        // An inherit-only entry does not apply to the object itself.
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        match u32::from(header.AceType) {
            ACCESS_ALLOWED_ACE_TYPE => {
                // SAFETY: the type byte says that the entry is an ACCESS_ALLOWED_ACE.
                let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
                let mut mask = allowed.Mask;
                // SAFETY: the mapping is a valid file generic mapping.
                unsafe { MapGenericMask(&mut mask, &mapping) };
                let granted = mask & rights;
                // SAFETY: SidStart is the first byte of the security identifier of the entry.
                let sid = std::ptr::addr_of!(allowed.SidStart).cast::<core::ffi::c_void>() as PSID;
                if granted != 0 && !is_trusted_sid(sid) {
                    return Ok(Some(granted));
                }
            }
            // An allow entry of a type this module does not parse may hold any right.
            ACCESS_ALLOWED_OBJECT_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE => {
                return Ok(Some(u32::MAX));
            }
            ACCESS_DENIED_ACE_TYPE
            | ACCESS_DENIED_OBJECT_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE
            | SYSTEM_AUDIT_ACE_TYPE
            | SYSTEM_AUDIT_OBJECT_ACE_TYPE
            | SYSTEM_ALARM_ACE_TYPE
            | SYSTEM_ALARM_OBJECT_ACE_TYPE
            | SYSTEM_MANDATORY_LABEL_ACE_TYPE
            | SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE
            | SYSTEM_SCOPED_POLICY_ID_ACE_TYPE => {}
            // Unknown types fail closed.
            _ => {
                return Ok(Some(u32::MAX));
            }
        }
    }
    Ok(None)
}

fn is_trusted_sid(sid: PSID) -> bool {
    for text in TRUSTED_SIDS {
        let Some(trusted) = sid_from_string(text) else {
            continue;
        };
        // SAFETY: both pointers are valid security identifiers.
        if unsafe { EqualSid(sid, trusted.0) } != 0 {
            return true;
        }
    }
    false
}

fn sid_from_string(text: &str) -> Option<OwnedSid> {
    let wide = wide_string(text);
    let mut sid: PSID = std::ptr::null_mut();
    // SAFETY: the text is a null-terminated UTF-16 string and the out parameter is a local.
    let ok = unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut sid) };
    if ok == 0 || sid.is_null() {
        None
    } else {
        Some(OwnedSid(sid))
    }
}

/// The file generic mapping, so a generic right is tested with the file access rights.
fn file_generic_mapping() -> GENERIC_MAPPING {
    GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ,
        GenericWrite: FILE_GENERIC_WRITE,
        GenericExecute: FILE_GENERIC_EXECUTE,
        GenericAll: FILE_ALL_ACCESS,
    }
}

fn create_dir(dir: &Path) -> Result<()> {
    let descriptor = descriptor(DIR_SDDL)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
            .context("the security attributes are too large")?,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let wide = wide_path(dir);
    // SAFETY: the path is a null-terminated UTF-16 string and the attributes stay alive for the call.
    let ok = unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) };
    if ok == 0 {
        bail!(
            "cannot create the directory {}: {}",
            dir.display(),
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn apply_descriptor(path: &Path, sddl: &str) -> Result<()> {
    let descriptor = descriptor(sddl)?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut defaulted = 0;
    // SAFETY: the descriptor is a valid security descriptor.
    unsafe { GetSecurityDescriptorOwner(descriptor.0, &mut owner, &mut defaulted) };
    let mut present = 0;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: the descriptor is a valid security descriptor.
    unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) };
    if present == 0 || dacl.is_null() {
        bail!(
            "the security descriptor of {} holds no DACL",
            path.display()
        );
    }
    let wide = wide_path(path);
    let info = OWNER_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | PROTECTED_DACL_SECURITY_INFORMATION;
    // SAFETY: the path is a null-terminated UTF-16 string; the owner and the DACL stay alive.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            info,
            owner,
            std::ptr::null_mut(),
            dacl,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        bail!(
            "cannot set the access control of {}: {}",
            path.display(),
            std::io::Error::from_raw_os_error(status as i32)
        );
    }
    Ok(())
}

fn descriptor(sddl: &str) -> Result<OwnedDescriptor> {
    let wide = wide_string(sddl);
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: the SDDL text is a null-terminated UTF-16 string and the out parameter is a local.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || descriptor.is_null() {
        bail!("cannot build the security descriptor");
    }
    Ok(OwnedDescriptor(descriptor))
}

fn wide_string(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub(crate) fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests;
