//! The registry value that keeps the time of a repair in place until a clean run after a restart.
//!
//! A handle that a process opened before the repair can reach any file of the program tree and of
//! `%ProgramData%\ResticHub`, so the value lives under `HKLM\SOFTWARE`, where only SYSTEM and
//! Administrators write.

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_QWORD, RRF_RT_REG_QWORD,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegGetValueW, RegSetValueExW,
};

const PARENT_KEY: &str = "SOFTWARE\\ResticHub";
pub(crate) const KEY: &str = "SOFTWARE\\ResticHub\\Agent";
pub(crate) const VALUE: &str = "ProgramRepairedInPlaceAt";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(status: u32) -> std::io::Error {
    std::io::Error::from_raw_os_error(status as i32)
}

/// The time of the repair in Unix seconds, or `None` without a record.
#[allow(unsafe_code)]
pub(crate) fn read() -> Result<Option<i64>> {
    let key = wide(KEY);
    let value = wide(VALUE);
    let mut data = 0_u64;
    let mut size = std::mem::size_of::<u64>() as u32;
    // SAFETY: both names end with a NUL, and `size` is the byte length of `data`.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_QWORD,
            std::ptr::null_mut(),
            std::ptr::from_mut(&mut data).cast(),
            &mut size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(i64::try_from(data).unwrap_or(i64::MAX))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        _ => bail!("cannot read HKLM\\{KEY}\\{VALUE}: {}", os_error(status)),
    }
}

#[allow(unsafe_code)]
pub(crate) fn write(repaired_at: i64) -> Result<()> {
    let key = wide(KEY);
    let mut handle: HKEY = std::ptr::null_mut();
    // SAFETY: the subkey ends with a NUL and the out parameter is a local.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut handle,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        bail!("cannot create HKLM\\{KEY}: {}", os_error(status));
    }
    let value = wide(VALUE);
    let data = u64::try_from(repaired_at).unwrap_or_default().to_le_bytes();
    // SAFETY: the key is open, the name ends with a NUL and the size is the length of `data`.
    let status = unsafe {
        RegSetValueExW(
            handle,
            value.as_ptr(),
            0,
            REG_QWORD,
            data.as_ptr(),
            data.len() as u32,
        )
    };
    // SAFETY: the key was opened above and is closed once.
    unsafe { RegCloseKey(handle) };
    if status != ERROR_SUCCESS {
        bail!("cannot write HKLM\\{KEY}\\{VALUE}: {}", os_error(status));
    }
    Ok(())
}

/// Removes the record and its key; the parent key goes only when nothing else is in it.
#[allow(unsafe_code)]
pub(crate) fn clear() -> Result<()> {
    let key = wide(KEY);
    // SAFETY: the subkey ends with a NUL.
    let status = unsafe { RegDeleteKeyW(HKEY_LOCAL_MACHINE, key.as_ptr()) };
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        bail!("cannot remove HKLM\\{KEY}: {}", os_error(status));
    }
    let parent = wide(PARENT_KEY);
    // SAFETY: the subkey ends with a NUL. A parent with other keys stays, which is no error.
    unsafe { RegDeleteKeyW(HKEY_LOCAL_MACHINE, parent.as_ptr()) };
    Ok(())
}

/// The start of the running Windows session in Unix seconds.
#[allow(unsafe_code)]
pub(crate) fn boot_time() -> i64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    // SAFETY: the call takes no argument.
    let uptime = unsafe { GetTickCount64() } / 1000;
    time::OffsetDateTime::now_utc().unix_timestamp() - i64::try_from(uptime).unwrap_or(0)
}
