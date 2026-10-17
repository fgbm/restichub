//! The processes that keep files open, as the Restart Manager reports them.

use std::os::windows::ffi::OsStrExt as _;
use std::path::PathBuf;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
    RmStartSession,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Holder {
    pub(crate) pid: u32,
    pub(crate) name: String,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (PID {})", self.name, self.pid)
    }
}

struct Session(u32);

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the session was started by `holders` and is ended once.
        unsafe { RmEndSession(self.0) };
    }
}

/// Returns the processes that keep one of `files` open. A handle to a directory, such as the
/// current directory of a shell, is not reported.
pub(crate) fn holders(files: &[PathBuf]) -> Result<Vec<Holder>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let mut key = [0_u16; CCH_RM_SESSION_KEY as usize + 1];
    let mut handle = 0;
    // SAFETY: the key buffer holds CCH_RM_SESSION_KEY characters and the terminator.
    let status = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
    if status != ERROR_SUCCESS {
        bail!(
            "cannot start a Restart Manager session: {}",
            std::io::Error::from_raw_os_error(status as i32)
        );
    }
    let session = Session(handle);
    let wide: Vec<Vec<u16>> = files
        .iter()
        .map(|file| file.as_os_str().encode_wide().chain(Some(0)).collect())
        .collect();
    let names: Vec<*const u16> = wide.iter().map(|name| name.as_ptr()).collect();
    let count = u32::try_from(names.len()).unwrap_or(u32::MAX);
    // SAFETY: each name ends with a NUL and outlives the call; no process or service is passed.
    let status = unsafe {
        RmRegisterResources(
            session.0,
            count,
            names.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        bail!(
            "cannot register the files with the Restart Manager: {}",
            std::io::Error::from_raw_os_error(status as i32)
        );
    }
    let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
    // The list can grow between the two calls, so the size is asked again.
    for _ in 0..4 {
        let mut needed = 0;
        let mut returned = u32::try_from(infos.len()).unwrap_or(u32::MAX);
        let mut reasons = 0;
        // SAFETY: the buffer holds `returned` entries, and a null buffer goes with a zero count.
        let status = unsafe {
            RmGetList(
                session.0,
                &mut needed,
                &mut returned,
                if infos.is_empty() {
                    std::ptr::null_mut()
                } else {
                    infos.as_mut_ptr()
                },
                &mut reasons,
            )
        };
        match status {
            ERROR_SUCCESS => {
                infos.truncate(returned as usize);
                return Ok(infos.iter().map(holder).collect());
            }
            ERROR_MORE_DATA => infos.resize(needed as usize, RM_PROCESS_INFO::default()),
            _ => bail!(
                "cannot read the list of the Restart Manager: {}",
                std::io::Error::from_raw_os_error(status as i32)
            ),
        }
    }
    bail!("the list of the Restart Manager kept changing")
}

fn holder(info: &RM_PROCESS_INFO) -> Holder {
    // The process sets its own name, and the text goes to the console of an administrator.
    let text = |units: &[u16]| {
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
            .chars()
            .map(|c| if c.is_control() { '?' } else { c })
            .collect::<String>()
    };
    let service = text(&info.strServiceShortName);
    let app = text(&info.strAppName);
    Holder {
        pid: info.Process.dwProcessId,
        name: match (app.is_empty(), service.is_empty()) {
            (false, true) => app,
            (false, false) => format!("{app}, service {service}"),
            (true, false) => format!("service {service}"),
            (true, true) => "an unnamed process".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::os::windows::fs::OpenOptionsExt as _;

    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

    use super::*;

    #[test]
    fn fr_in_07_the_process_that_keeps_a_file_open_is_named() {
        let dir = crate::winsec::tests::temp("holders");
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        let file = dir.join("held.txt");
        std::fs::write(&file, b"x").expect("cannot write");
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&file)
            .expect("cannot open");
        let found = holders(std::slice::from_ref(&file)).expect("the list must be read");
        assert!(
            found.iter().any(|holder| holder.pid == std::process::id()),
            "{found:?}"
        );
        drop(held);
        let found = holders(std::slice::from_ref(&file)).expect("the list must be read");
        assert!(found.is_empty(), "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_in_07_a_holder_name_reaches_the_console_without_control_characters() {
        let mut info = RM_PROCESS_INFO::default();
        for (slot, unit) in info
            .strAppName
            .iter_mut()
            .zip("tool\u{1b}[2J\r\nname".encode_utf16())
        {
            *slot = unit;
        }
        info.Process.dwProcessId = 7;
        assert_eq!(holder(&info).to_string(), "tool?[2J??name (PID 7)");
    }
}
