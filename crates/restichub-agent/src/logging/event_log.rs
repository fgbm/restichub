//! The Application event log source of the Windows service.
//!
//! The source has no message file of its own. It borrows the message table of `EventCreate.exe`,
//! which every Windows edition holds and whose messages 1 to 1000 are the bare insertion string
//! `%1`. So Event Viewer shows the text of the agent instead of "the description cannot be found".

#![allow(unsafe_code)]

use anyhow::{Result, bail};
use tracing::Level;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::System::EventLog::{
    DeregisterEventSource, EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE,
    RegisterEventSourceW, ReportEventW,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_DWORD, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegSetValueExW,
};

/// The name that Event Viewer shows and that `Get-WinEvent -ProviderName` takes.
pub const SOURCE: &str = "ResticHub Agent";

const SOURCE_KEY: &str = r"SYSTEM\CurrentControlSet\Services\EventLog\Application\ResticHub Agent";

const MESSAGE_FILE: &str = r"%SystemRoot%\System32\EventCreate.exe";

const ERROR_EVENT_ID: u32 = 1;
const WARNING_EVENT_ID: u32 = 2;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(super) struct EventSource(HANDLE);

// SAFETY: an event log handle may be used from any thread.
unsafe impl Send for EventSource {}
// SAFETY: `ReportEventW` is thread-safe for one handle.
unsafe impl Sync for EventSource {}

impl EventSource {
    pub(super) fn open() -> Result<Self> {
        let source = wide(SOURCE);
        // SAFETY: a null server name means the local computer; the source name ends with a NUL.
        let handle = unsafe { RegisterEventSourceW(std::ptr::null(), source.as_ptr()) };
        if handle.is_null() {
            bail!(
                "cannot open the event source {SOURCE}: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Self(handle))
    }
}

impl Drop for EventSource {
    fn drop(&mut self) {
        // SAFETY: the handle came from `RegisterEventSourceW` and is closed once.
        unsafe { DeregisterEventSource(self.0) };
    }
}

impl super::sinks::EventSink for EventSource {
    fn report(&self, level: Level, text: &str) -> std::io::Result<()> {
        let (kind, id) = if level == Level::ERROR {
            (EVENTLOG_ERROR_TYPE, ERROR_EVENT_ID)
        } else {
            (EVENTLOG_WARNING_TYPE, WARNING_EVENT_ID)
        };
        let text = wide(text);
        let strings = [text.as_ptr()];
        // SAFETY: the handle is open, and the one string ends with a NUL and lives until the call
        // returns.
        let ok = unsafe {
            ReportEventW(
                self.0,
                kind,
                0,
                id,
                std::ptr::null_mut(),
                1,
                0,
                strings.as_ptr(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Registers the event source of the service in the Application log (`install-service`).
///
/// # Errors
///
/// Returns an error when the registry key cannot be written, usually without administrator
/// rights.
pub fn register_event_source() -> Result<()> {
    let key = wide(SOURCE_KEY);
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
        bail!(
            "cannot register the event source {SOURCE}: {}",
            std::io::Error::from_raw_os_error(status as i32)
        );
    }
    let message_file = wide(MESSAGE_FILE);
    let message_bytes: Vec<u8> = message_file
        .iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let types: u32 =
        u32::from(EVENTLOG_ERROR_TYPE | EVENTLOG_WARNING_TYPE | EVENTLOG_INFORMATION_TYPE);
    let result = set_value(handle, "EventMessageFile", REG_EXPAND_SZ, &message_bytes)
        .and_then(|()| set_value(handle, "TypesSupported", REG_DWORD, &types.to_le_bytes()));
    // SAFETY: the key was opened above and is closed once.
    unsafe { RegCloseKey(handle) };
    result
}

fn set_value(key: HKEY, name: &str, kind: u32, data: &[u8]) -> Result<()> {
    let name_wide = wide(name);
    let size = u32::try_from(data.len())?;
    // SAFETY: the key is open, the name ends with a NUL and `size` is the length of `data`.
    let status = unsafe { RegSetValueExW(key, name_wide.as_ptr(), 0, kind, data.as_ptr(), size) };
    if status != ERROR_SUCCESS {
        bail!(
            "cannot write {name} of the event source {SOURCE}: {}",
            std::io::Error::from_raw_os_error(status as i32)
        );
    }
    Ok(())
}

/// Removes the event source of the service (`uninstall-service`). The events stay in the log.
///
/// # Errors
///
/// Returns an error when the registry key exists and cannot be removed.
pub fn remove_event_source() -> Result<()> {
    let key = wide(SOURCE_KEY);
    // SAFETY: the subkey ends with a NUL.
    let status = unsafe { RegDeleteKeyW(HKEY_LOCAL_MACHINE, key.as_ptr()) };
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        bail!(
            "cannot remove the event source {SOURCE}: {}",
            std::io::Error::from_raw_os_error(status as i32)
        );
    }
    Ok(())
}
