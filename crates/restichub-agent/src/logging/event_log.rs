//! The `ResticHub` event log and its source `ResticHub Agent`.
//!
//! The agent has a classic log of its own, so that only SYSTEM, Administrators and Event Log Readers
//! read its events and only SYSTEM and Administrators write events with its source name. In the
//! Application log any local user could do both.
//!
//! The source has no message file of its own. It borrows the message table of `EventCreate.exe`,
//! which every Windows edition holds and whose messages 1 to 1000 are the bare insertion string
//! `%1`. So Event Viewer shows the text of the agent instead of "the description cannot be found".

#![allow(unsafe_code)]

use anyhow::{Context as _, Result, bail};
use tracing::Level;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::System::EventLog::{
    DeregisterEventSource, EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE,
    RegisterEventSourceW, ReportEventW,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_CREATED_NEW_KEY, REG_DWORD,
    REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW,
    RegOpenKeyExW, RegSetValueExW,
};

/// The log that Event Viewer shows under "Applications and Services Logs".
pub const LOG: &str = "ResticHub";

/// The name that Event Viewer shows and that `Get-WinEvent -ProviderName` takes.
pub const SOURCE: &str = "ResticHub Agent";

const LOG_KEY: &str = r"SYSTEM\CurrentControlSet\Services\EventLog\ResticHub";

const SOURCE_KEY: &str = r"SYSTEM\CurrentControlSet\Services\EventLog\ResticHub\ResticHub Agent";

/// Read, write and clear (0x7) for SYSTEM and Administrators, read (0x1) for Event Log Readers,
/// as the Application log grants them. No other account reads or writes the log.
const CHANNEL_ACCESS: &str = "O:BAG:SYD:(A;;0xf0007;;;SY)(A;;0x7;;;BA)(A;;0x1;;;S-1-5-32-573)";

const LOG_FILE: &str = r"%SystemRoot%\System32\Winevt\Logs\ResticHub.evtx";

/// The default size of the Application log. An administrator can change it; a new install-service
/// keeps the change.
const MAX_LOG_BYTES: u32 = 20 * 1024 * 1024;

const MESSAGE_FILE: &str = r"%SystemRoot%\System32\EventCreate.exe";

const ERROR_EVENT_ID: u32 = 1;
const WARNING_EVENT_ID: u32 = 2;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(status: u32) -> std::io::Error {
    std::io::Error::from_raw_os_error(i32::try_from(status).unwrap_or(i32::MAX))
}

pub(super) struct EventSource(HANDLE);

// SAFETY: an event log handle may be used from any thread.
unsafe impl Send for EventSource {}
// SAFETY: `ReportEventW` is thread-safe for one handle.
unsafe impl Sync for EventSource {}

impl EventSource {
    /// Opens the source only when `install-service` registered it: Windows writes the events of an
    /// unknown source to the Application log.
    pub(super) fn open() -> Result<Self> {
        drop(Key::open(SOURCE_KEY, KEY_QUERY_VALUE).with_context(|| {
            format!("the event source {SOURCE} is not registered; run install-service")
        })?);
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

struct Key {
    handle: HKEY,
    created: bool,
}

impl Key {
    fn create(path: &str) -> std::io::Result<Self> {
        let path = wide(path);
        let mut handle: HKEY = std::ptr::null_mut();
        let mut disposition = 0;
        // SAFETY: the subkey ends with a NUL and the out parameters are locals.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                path.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                &mut disposition,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(os_error(status));
        }
        Ok(Self {
            handle,
            created: disposition == REG_CREATED_NEW_KEY,
        })
    }

    fn open(path: &str, access: u32) -> std::io::Result<Self> {
        let path = wide(path);
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: the subkey ends with a NUL and the out parameter is a local.
        let status =
            unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, access, &mut handle) };
        if status != ERROR_SUCCESS {
            return Err(os_error(status));
        }
        Ok(Self {
            handle,
            created: false,
        })
    }

    fn set_text(&self, name: &str, kind: u32, value: &str) -> Result<()> {
        let bytes: Vec<u8> = wide(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        self.set(name, kind, &bytes)
    }

    fn set(&self, name: &str, kind: u32, data: &[u8]) -> Result<()> {
        let name_wide = wide(name);
        let size = u32::try_from(data.len())?;
        // SAFETY: the key is open, the name ends with a NUL and `size` is the length of `data`.
        let status = unsafe {
            RegSetValueExW(
                self.handle,
                name_wide.as_ptr(),
                0,
                kind,
                data.as_ptr(),
                size,
            )
        };
        if status != ERROR_SUCCESS {
            bail!("cannot write {name}: {}", os_error(status));
        }
        Ok(())
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key was opened by this value and is closed once.
        unsafe { RegCloseKey(self.handle) };
    }
}

fn delete_key(path: &str) -> std::io::Result<()> {
    let path = wide(path);
    // SAFETY: the subkey ends with a NUL.
    let status = unsafe { RegDeleteKeyW(HKEY_LOCAL_MACHINE, path.as_ptr()) };
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        return Err(os_error(status));
    }
    Ok(())
}

/// Writes the registry keys of the `ResticHub` log and its source (`install-service`).
///
/// A failure removes the keys that this call created, so the registry is never half written.
///
/// # Errors
///
/// Returns an error when a registry key cannot be written, usually without administrator rights.
pub fn register_event_source() -> Result<()> {
    let log =
        Key::create(LOG_KEY).with_context(|| format!("cannot register the event log {LOG}"))?;
    let mut source = None;
    let result = write_log_values(&log).and_then(|()| {
        let key = Key::create(SOURCE_KEY)?;
        let written = write_source_values(&key);
        source = Some(key);
        written
    });
    let Err(error) = result else {
        return Ok(());
    };
    if let Some(source) = source.take().filter(|key| key.created) {
        drop(source);
        let _ = delete_key(SOURCE_KEY);
    }
    if log.created {
        drop(log);
        let _ = delete_key(LOG_KEY);
    }
    Err(error.context(format!(
        "cannot register the event log {LOG} and the source {SOURCE}"
    )))
}

fn write_log_values(log: &Key) -> Result<()> {
    log.set_text("CustomSD", REG_SZ, CHANNEL_ACCESS)?;
    if log.created {
        log.set_text("File", REG_EXPAND_SZ, LOG_FILE)?;
        log.set("MaxSize", REG_DWORD, &MAX_LOG_BYTES.to_le_bytes())?;
        log.set("Retention", REG_DWORD, &0u32.to_le_bytes())?;
    }
    Ok(())
}

fn write_source_values(source: &Key) -> Result<()> {
    let types: u32 =
        u32::from(EVENTLOG_ERROR_TYPE | EVENTLOG_WARNING_TYPE | EVENTLOG_INFORMATION_TYPE);
    source.set_text("EventMessageFile", REG_EXPAND_SZ, MESSAGE_FILE)?;
    source.set("TypesSupported", REG_DWORD, &types.to_le_bytes())
}

/// Removes the event source (`uninstall-service`). The `ResticHub` log and its events stay.
///
/// # Errors
///
/// Returns an error when the registry key exists and cannot be removed.
pub fn remove_event_source() -> Result<()> {
    delete_key(SOURCE_KEY).with_context(|| format!("cannot remove the event source {SOURCE}"))
}
