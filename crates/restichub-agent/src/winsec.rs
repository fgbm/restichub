//! The protected directory and the file access control on Windows (FR-AG-04, threat T20).
//!
//! The identity must stay on SYSTEM and on Administrators. The module builds an explicit security
//! descriptor and applies it with the Windows API, so no inherited access control entry (a normal
//! `%ProgramData%` grants a normal user a read) reaches the key. A directory that already exists
//! with another owner, or with a write access for another account, is refused before any file is
//! written.

#![allow(unsafe_code)]

use std::io::Write as _;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
    OBJECT_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
};
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

/// A protected DACL: full control for SYSTEM and Administrators only, inherited by the children,
/// with Administrators as the owner.
const DIR_SDDL: &str = "O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// A protected DACL for one file: full control for SYSTEM and Administrators only.
const FILE_SDDL: &str = "O:BAG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)";

/// The write rights of a file access mask: data, attributes, delete and the access control.
const WRITE_MASK: u32 = 0x0045_0116;

/// Owns a security descriptor and frees it on drop.
struct OwnedDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for OwnedDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0 as HLOCAL) };
        }
    }
}

/// Makes `dir` and its parent with a protected DACL for SYSTEM and Administrators (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the directory or its parent cannot be created, when it already exists with
/// another owner or with a write access for another account, or when the access control cannot be
/// set.
pub fn ensure_config_dir(dir: &Path) -> Result<()> {
    if let Some(parent) = dir.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        create_protected_dir(parent)
            .with_context(|| format!("cannot protect the parent directory {}", parent.display()))?;
    }
    create_protected_dir(dir)
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

/// Refuses an existing path that SYSTEM and Administrators do not own alone.
///
/// # Errors
///
/// Returns an error when another account owns the path or holds a write access on it.
pub fn check_admin_only(path: &Path) -> Result<()> {
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
            "cannot read the access control of {} (error {status})",
            path.display()
        );
    }
    let descriptor = OwnedDescriptor(descriptor);
    let sddl = to_sddl(descriptor.0, info)?;
    check_sddl(path, &sddl)
}

fn check_sddl(path: &Path, sddl: &str) -> Result<()> {
    let owner = component(sddl, 'O').unwrap_or_default();
    if !is_system_or_admin(owner) {
        bail!(
            "{} is owned by {owner}, not by SYSTEM or Administrators; an administrator deletes it and registers again",
            path.display()
        );
    }
    for (trustee, rights) in aces(sddl) {
        if is_system_or_admin(&trustee) {
            continue;
        }
        if grants_write(&rights) {
            bail!(
                "{} grants {rights} to {trustee}; an administrator deletes it and registers again",
                path.display()
            );
        }
    }
    Ok(())
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
        bail!("cannot create the directory {}", dir.display());
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
            "cannot set the access control of {} (error {status}); run the command as an administrator",
            path.display()
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

fn to_sddl(descriptor: PSECURITY_DESCRIPTOR, info: OBJECT_SECURITY_INFORMATION) -> Result<String> {
    let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
    let mut length = 0_u32;
    // SAFETY: the descriptor is valid and the out parameters are locals.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            info,
            &mut text,
            &mut length,
        )
    };
    if ok == 0 || text.is_null() {
        bail!("cannot read the security descriptor as text");
    }
    // SAFETY: the call returned a null-terminated UTF-16 string of `length` units.
    let value = unsafe { utf16_to_string(text, length as usize) };
    // SAFETY: the string came from the conversion function and needs LocalFree.
    unsafe { LocalFree(text as HLOCAL) };
    Ok(value)
}

/// Reads a null-terminated UTF-16 string of at most `length` units.
unsafe fn utf16_to_string(pointer: *const u16, length: usize) -> String {
    let slice = unsafe { std::slice::from_raw_parts(pointer, length) };
    let end = slice
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(slice.len());
    String::from_utf16_lossy(&slice[..end])
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

/// The value of one SDDL component, for example `BA` in `O:BA`.
///
/// The components are concatenated without a separator: the owner ends where the next tag starts.
fn component<'a>(sddl: &'a str, tag: char) -> Option<&'a str> {
    components(sddl)
        .into_iter()
        .find(|(found, _)| *found == tag)
        .map(|(_, value)| value)
}

/// The `(tag, value)` of every top-level SDDL component, for example `('O', "BA")`.
///
/// A tag is one of `O`, `G`, `D` and `S` followed by a colon. A value holds no colon, so the next
/// tag begins after the value.
fn components(sddl: &str) -> Vec<(char, &str)> {
    let bytes = sddl.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index + 1 < bytes.len() {
        let tag = bytes[index] as char;
        if bytes[index + 1] != b':' || !matches!(tag, 'O' | 'G' | 'D' | 'S') {
            index += 1;
            continue;
        }
        let start = index + 2;
        let mut end = sddl.len();
        let mut scan = start;
        while scan + 1 < bytes.len() {
            let next = bytes[scan] as char;
            if bytes[scan + 1] == b':' && matches!(next, 'O' | 'G' | 'D' | 'S') {
                end = scan;
                break;
            }
            scan += 1;
        }
        found.push((tag, &sddl[start..end]));
        index = end;
    }
    found
}

/// The `(rights, trustee)` pair of every access control entry in the DACL.
fn aces(sddl: &str) -> Vec<(String, String)> {
    let mut rest = component(sddl, 'D')
        .unwrap_or_default()
        .trim_start_matches('P');
    let mut entries = Vec::new();
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let fields: Vec<&str> = rest[open + 1..open + close].split(';').collect();
        if fields.len() >= 6 {
            entries.push((fields[fields.len() - 1].to_owned(), fields[2].to_owned()));
        }
        rest = &rest[open + close + 1..];
    }
    entries
}

fn is_system_or_admin(sid: &str) -> bool {
    match sid {
        "SY" | "S-1-5-18" => true,
        "BA" | "S-1-5-32-544" => true,
        // CREATOR OWNER and OWNER RIGHTS resolve to the owner of the object, which must be SYSTEM
        // or Administrators itself. A standard `%ProgramFiles%` holds both a read for Users, which
        // the caller ignores, and a full control for TrustedInstaller, below.
        "CO" | "OW" | "S-1-3-0" | "S-1-3-4" => true,
        // TrustedInstaller owns and controls a path that the Windows installer created.
        _ => sid.starts_with("S-1-5-80-"),
    }
}

/// True when a rights string allows a change to the file.
fn grants_write(rights: &str) -> bool {
    let rights = rights.trim();
    if let Some(hex) = rights
        .strip_prefix("0x")
        .or_else(|| rights.strip_prefix("0X"))
    {
        // An unknown mask is treated as a write, so a bad value never widens the access.
        return u32::from_str_radix(hex, 16).map_or(true, |mask| mask & WRITE_MASK != 0);
    }
    // The short forms: `FA` and `GA` mean all, and a `W` marks a write right.
    rights.contains('W') || rights.eq_ignore_ascii_case("FA") || rights.eq_ignore_ascii_case("GA")
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
    fn a_directory_with_another_owner_is_refused() {
        // The base is SYSTEM and Administrators only, so the refusal names the inner directory.
        let base = temp("owner-base");
        create_protected_dir(&base).expect("cannot create the protected base");
        let dir = base.join("agent");
        create_protected_dir(&dir).expect("cannot create the protected directory");
        // An administrator owns a new directory through Administrators, so the test sets the owner
        // to the account of the current process, which is not SYSTEM or Administrators.
        let sid = current_user_sid().expect("cannot read the user SID");
        set_owner(&dir, &sid).expect("cannot set the owner");
        assert!(ensure_config_dir(&dir).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_directory_with_a_write_ace_for_everyone_is_refused() {
        let dir = temp("acl");
        create_protected_dir(&dir).expect("cannot create the protected directory");
        grant_everyone(&dir).expect("cannot add the access control entry");
        assert!(check_admin_only(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
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

    /// Adds a full-control access control entry for every account, for a negative test.
    fn grant_everyone(dir: &Path) -> Result<()> {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
        let icacls = Path::new(&system_root).join("System32").join("icacls.exe");
        let status = std::process::Command::new(icacls)
            .arg(dir)
            .args(["/grant", "*S-1-1-0:(OI)(CI)F"])
            .status()
            .context("cannot run icacls")?;
        if !status.success() {
            bail!("icacls failed with {status}");
        }
        Ok(())
    }

    #[test]
    fn the_sddl_parser_reads_the_owner_and_the_entries() {
        let sddl = "O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
        assert_eq!(component(sddl, 'O'), Some("BA"));
        let entries = aces(sddl);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "SY");
        assert!(grants_write(&entries[0].1));
        assert!(!grants_write("0x120089"));
    }
}
