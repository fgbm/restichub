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
use std::path::Path;

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
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
    CreateDirectoryW, DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ALL_ACCESS,
    FILE_APPEND_DATA, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA,
    WRITE_DAC, WRITE_OWNER,
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
    protect_tree(dir, root, &mut |level| {
        create_dir(level)?;
        apply_descriptor(level, DIR_SDDL)
    })
}

fn protect_tree(
    dir: &Path,
    root: &Path,
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
                check_admin_only(level)
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
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("cannot flush {}", path.display()))?;
    drop(file);
    apply_descriptor(path, FILE_SDDL)
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

fn check(path: &Path, rights: u32) -> Result<()> {
    let security = open_security(path)?;
    check_owner(path, security.owner)?;
    if let Some((mask, account)) = first_foreign_grant(security.dacl, rights)? {
        bail!(
            "{} grants the access mask {mask:#010x} to {account}, which is not SYSTEM or Administrators; an administrator resets the owner and the access to SYSTEM and Administrators, then runs the command again",
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
        bail!(
            "{} is owned by {}, not by SYSTEM or Administrators; an administrator resets the owner and the access to SYSTEM and Administrators, then runs the command again",
            path.display(),
            sid_to_string(owner)
        );
    }
    Ok(())
}

/// Returns the mask that a non-admin holds, or `None` when no entry grants it.
///
/// A missing DACL grants everyone full access, so it gives [`u32::MAX`]. An allow entry of a type
/// this module does not parse also gives [`u32::MAX`], because it may hold any right.
fn first_foreign_grant(dacl: *mut ACL, rights: u32) -> Result<Option<(u32, String)>> {
    if dacl.is_null() {
        return Ok(Some((u32::MAX, "every account".to_owned())));
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
                    return Ok(Some((granted, sid_to_string(sid))));
                }
            }
            // An allow entry of a type this module does not parse may hold any right.
            ACCESS_ALLOWED_OBJECT_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE => {
                return Ok(Some((
                    u32::MAX,
                    "an unparsed access control entry".to_owned(),
                )));
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
                return Ok(Some((
                    u32::MAX,
                    "an unparsed access control entry".to_owned(),
                )));
            }
        }
    }
    Ok(None)
}

/// The text form of a security identifier, for a message.
fn sid_to_string(sid: PSID) -> String {
    if sid.is_null() {
        return "an unknown account".to_owned();
    }
    let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
    // SAFETY: the SID is valid and the out parameter is a local.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut text) };
    if ok == 0 || text.is_null() {
        return "an unknown account".to_owned();
    }
    // SAFETY: the call returned a null-terminated UTF-16 string.
    let value = unsafe { utf16_string(text) };
    // SAFETY: the string came from the conversion function and needs LocalFree.
    unsafe { LocalFree(text as HLOCAL) };
    value
}

/// Reads a null-terminated UTF-16 value.
unsafe fn utf16_string(pointer: *const u16) -> String {
    let mut length = 0;
    // SAFETY: the caller gives a null-terminated string.
    while unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the length counts the units before the terminator.
    let slice = unsafe { std::slice::from_raw_parts(pointer, length) };
    String::from_utf16_lossy(slice)
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
            "cannot set the access control of {}: {} (error {status}); run the command as an administrator",
            path.display(),
            std::io::Error::last_os_error()
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

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "restichub-agent-winsec-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_directory_that_another_account_owns_is_refused() {
        // The base is SYSTEM and Administrators only, so the refusal names the inner directory.
        let base = temp("owner-base");
        create_protected_dir(&base).expect("cannot create the protected base");
        let dir = base.join("agent");
        create_protected_dir(&dir).expect("cannot create the protected directory");
        // An administrator owns a new directory through Administrators, so the test sets the owner
        // to the account of the current process, which is not SYSTEM or Administrators.
        let sid = current_user_sid().expect("cannot read the user SID");
        set_owner(&dir, &sid).expect("cannot set the owner");
        assert!(check_admin_only(&dir).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_directory_with_a_write_ace_for_users_is_refused() {
        let dir = temp("acl-write");
        create_protected_dir(&dir).expect("cannot create the protected directory");
        grant(&dir, "(OI)(CI)(WD)").expect("cannot add the access control entry");
        assert!(check_admin_only(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_with_a_write_owner_ace_for_users_is_refused() {
        let dir = temp("acl-owner");
        create_protected_dir(&dir).expect("cannot create the protected directory");
        grant(&dir, "(OI)(CI)(WO)").expect("cannot add the access control entry");
        assert!(check_admin_only(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ac_hk_01_a_program_directory_where_users_add_files_is_refused() {
        // `(W)` grants write data and attributes: the right to add a file, without a delete.
        for rights in ["(W)", "(OI)(CI)(WD)", "(AD)"] {
            let dir = temp("program-dir");
            create_protected_dir(&dir).expect("cannot create the protected directory");
            check_program_directory(&dir).expect("a protected directory must pass");
            grant(&dir, rights).expect("cannot add the access control entry");
            assert!(check_program_directory(&dir).is_err(), "{rights}");
            // A parent check accepts the same directory, so it cannot guard a program directory.
            if rights == "(W)" {
                check_parent(&dir).expect("a parent may let users add files");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn fr_ag_04_a_tree_with_two_missing_levels_is_created_protected() {
        let base = temp("tree-base");
        std::fs::create_dir_all(&base).expect("cannot create the base");
        let root = base.join("ResticHub");
        let dir = root.join("Agent").join("cache");
        create_protected_tree(&dir, &root).expect("cannot create the tree");
        for level in [&root, &root.join("Agent"), &dir] {
            check_admin_only(level)
                .expect("a created level must be SYSTEM and Administrators only");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_tree_below_a_user_writable_level_is_refused() {
        let base = temp("tree-writable");
        std::fs::create_dir_all(&base).expect("cannot create the base");
        let root = base.join("ResticHub");
        create_protected_dir(&root).expect("cannot create the protected directory");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the access control entry");
        let dir = root.join("Agent").join("cache");
        assert!(create_protected_tree(&dir, &root).is_err());
        assert!(!root.join("Agent").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_existing_level_is_not_protected_again() {
        let base = temp("tree-twice");
        std::fs::create_dir_all(&base).expect("cannot create the base");
        let root = base.join("ResticHub");
        let dir = root.join("Agent").join("cache");
        create_protected_tree(&dir, &root).expect("cannot create the tree");
        let mut created = Vec::new();
        protect_tree(&dir, &root, &mut |level| {
            created.push(level.to_path_buf());
            Ok(())
        })
        .expect("cannot check the existing tree");
        assert!(created.is_empty(), "an existing level was created again");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_the_security_of_an_existing_child_stays_after_a_second_call() {
        let base = temp("tree-child");
        std::fs::create_dir_all(&base).expect("cannot create the base");
        let root = base.join("ResticHub");
        let dir = root.join("cache");
        create_protected_tree(&dir, &root).expect("cannot create the tree");
        let child = dir.join("data");
        create_protected_file(&child, b"x").expect("cannot write the child");
        let before = std::fs::metadata(&child)
            .and_then(|metadata| metadata.modified())
            .expect("cannot read the child");
        create_protected_tree(&dir, &root).expect("cannot check the tree again");
        let after = std::fs::metadata(&child)
            .and_then(|metadata| metadata.modified())
            .expect("cannot read the child");
        assert_eq!(before, after);
        check_admin_only(&child).expect("the child must stay SYSTEM and Administrators only");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_junction_level_is_refused() {
        let base = temp("tree-junction");
        let root = base.join("ResticHub");
        let target = base.join("target");
        std::fs::create_dir_all(&target).expect("cannot create the target");
        create_protected_dir(&root).expect("cannot create the protected root");
        let junction = root.join("Agent");
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&junction)
            .arg(&target)
            .status()
            .expect("cannot run mklink");
        assert!(status.success(), "cannot create the junction");
        let error = create_protected_tree(&junction.join("cache"), &root)
            .expect_err("a junction level must be refused");
        assert!(format!("{error:#}").contains("reparse point"), "{error:#}");
        assert!(!target.join("cache").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_plain_file_is_refused() {
        let file = temp("plain.pem");
        std::fs::write(&file, b"key").expect("cannot write the file");
        assert!(check_admin_only(&file).is_err());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn the_key_file_is_admin_only() {
        let dir = temp("dir");
        create_protected_dir(&dir).expect("cannot create the protected directory");
        let file = dir.join("agent-key.pem");
        create_protected_file(&file, b"key").expect("cannot write the protected file");
        check_admin_only(&file).expect("the key file must be SYSTEM and Administrators only");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_program_files_acl_passes_the_agent_rule() {
        // The real Program Files DACL grants Users a read and TrustedInstaller a full control.
        let dir = temp("program-files-acl");
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        copy_program_files_dacl(&dir).expect("cannot copy the DACL");
        check_admin_only(&dir).expect("the Program Files DACL must pass");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_program_files_parent_chain_passes() {
        let mut current = Some(std::path::Path::new(r"C:\Program Files"));
        while let Some(dir) = current {
            check_parent(dir)
                .unwrap_or_else(|error| panic!("{} must pass: {error}", dir.display()));
            current = dir.parent();
        }
    }

    /// Reads the security identifier of the account that runs the process.
    fn current_user_sid() -> Result<String> {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
        let whoami = Path::new(&system_root).join("System32").join("whoami.exe");
        let output = std::process::Command::new(whoami)
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .context("cannot run whoami")?;
        let text = String::from_utf8_lossy(&output.stdout);
        // The line is `"domain\account","S-1-5-21-..."`.
        let sid = text
            .rsplit("\",\"")
            .next()
            .unwrap_or_default()
            .trim_matches(|character| character == '"' || character == '\r' || character == '\n')
            .to_owned();
        if !sid.starts_with("S-1-") {
            bail!("cannot read the user SID from `{text}`");
        }
        Ok(sid)
    }

    /// Sets the owner of `path` to the SDDL identity, for a negative test.
    fn set_owner(path: &Path, sid: &str) -> Result<()> {
        let descriptor = descriptor(&format!("O:{sid}"))?;
        let mut owner: PSID = std::ptr::null_mut();
        let mut defaulted = 0;
        // SAFETY: the descriptor is a valid security descriptor.
        unsafe { GetSecurityDescriptorOwner(descriptor.0, &mut owner, &mut defaulted) };
        let wide = wide_path(path);
        // SAFETY: the path is a null-terminated UTF-16 string and the owner stays alive.
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                owner,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if status != ERROR_SUCCESS {
            bail!("cannot set the owner (error {status})");
        }
        Ok(())
    }

    /// Grants the account of BUILTIN\Users the given rights, for a negative test.
    fn grant(dir: &Path, rights: &str) -> Result<()> {
        run_icacls(dir, &["/grant", &format!("*S-1-5-32-545:{rights}")])
    }

    /// Copies the DACL of the real `C:\Program Files` onto `dir`, for a positive test.
    fn copy_program_files_dacl(dir: &Path) -> Result<()> {
        let source = open_security(Path::new(r"C:\Program Files"))?;
        let owner = sid_from_string("S-1-5-32-544").context("cannot build the owner SID")?;
        let wide = wide_path(dir);
        let info = OWNER_SECURITY_INFORMATION
            | DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION;
        // SAFETY: the path is a null-terminated UTF-16 string and the DACL stays alive.
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                info,
                owner.0,
                std::ptr::null_mut(),
                source.dacl,
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            bail!("cannot copy the DACL (error {status})");
        }
        Ok(())
    }

    /// Runs `%SystemRoot%\System32\icacls.exe` by absolute path.
    fn run_icacls(dir: &Path, arguments: &[&str]) -> Result<()> {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
        let icacls = Path::new(&system_root).join("System32").join("icacls.exe");
        let status = std::process::Command::new(icacls)
            .arg(dir)
            .args(arguments)
            .status()
            .context("cannot run icacls")?;
        if !status.success() {
            bail!("icacls failed with {status}");
        }
        Ok(())
    }
}
