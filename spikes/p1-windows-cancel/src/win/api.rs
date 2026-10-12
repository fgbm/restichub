//! Thin wrappers around the Win32 calls that the spike needs.
//!
//! The functions return `io::Result`. The caller records the error code in the
//! report.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::Child as StdChild;

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    AllocConsole, AttachConsole, CTRL_BREAK_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
    GetConsoleWindow, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, PROCESS_INFORMATION, STARTF_USESHOWWINDOW,
    STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};

/// A job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
///
/// Every child of the spike goes into a job. When the spike exits, the job
/// handle closes and Windows kills the children.
pub struct JobObject(HANDLE);

impl JobObject {
    pub fn create() -> io::Result<Self> {
        // SAFETY: The call passes null attributes and a null name. The handle
        // gets closed in `Drop` or directly on the error path.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast::<c_void>(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let error = io::Error::last_os_error();
                CloseHandle(handle);
                return Err(error);
            }
            Ok(JobObject(handle))
        }
    }

    pub fn assign(&self, process: HANDLE) -> io::Result<()> {
        // SAFETY: Both handles are valid. The job lives longer than the call.
        let ok = unsafe { AssignProcessToJobObject(self.0, process) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        // SAFETY: The job handle is valid until `Drop`.
        let ok = unsafe { TerminateJobObject(self.0, exit_code) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: The handle comes from `CreateJobObjectW` and is closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Allocate a console for the current process.
pub fn alloc_console() -> io::Result<()> {
    // SAFETY: `AllocConsole` takes no arguments.
    let ok = unsafe { AllocConsole() };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Hide the console window of the current process.
pub fn hide_console_window() -> io::Result<()> {
    // SAFETY: `GetConsoleWindow` returns a window handle or null.
    let hwnd = unsafe { GetConsoleWindow() };
    if hwnd.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the process has no console window",
        ));
    }
    // SAFETY: The handle is the console window of this process.
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
    }
    Ok(())
}

/// Send `CTRL_BREAK_EVENT` to a process group.
pub fn generate_break(process_group: u32) -> io::Result<()> {
    // SAFETY: `GenerateConsoleCtrlEvent` reads only its arguments.
    let ok = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, process_group) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Detach the current process from its console.
pub fn free_console() {
    // SAFETY: `FreeConsole` takes no arguments. The error is not important.
    unsafe {
        FreeConsole();
    }
}

/// Attach the current process to the console of another process.
pub fn attach_console(pid: u32) -> io::Result<()> {
    // SAFETY: `AttachConsole` reads only its argument.
    let ok = unsafe { AttachConsole(pid) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Make the current process ignore `CTRL_C_EVENT`.
pub fn ignore_ctrl_c() {
    // SAFETY: A null handler with `add = TRUE` sets the ignore flag.
    unsafe {
        SetConsoleCtrlHandler(None, 1);
    }
}

/// A running child process. It wraps a `std::process::Child` or a raw process
/// handle from a direct `CreateProcessW` call.
pub struct RunningChild {
    kind: ChildKind,
    pid: u32,
}

enum ChildKind {
    Std(StdChild),
    Raw(HANDLE),
}

impl RunningChild {
    pub fn from_std(child: StdChild) -> Self {
        let pid = child.id();
        RunningChild {
            kind: ChildKind::Std(child),
            pid,
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The process handle, used for the job object assignment.
    pub fn handle(&self) -> HANDLE {
        match &self.kind {
            ChildKind::Std(child) => child.as_raw_handle(),
            ChildKind::Raw(handle) => *handle,
        }
    }

    /// Stop the child immediately. The caller uses this call after a
    /// `try_wait` timeout, so that a stuck helper cannot block the harness.
    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.kind {
            ChildKind::Std(child) => child.kill(),
            // SAFETY: The handle is a valid process handle.
            ChildKind::Raw(handle) => {
                let ok = unsafe { TerminateProcess(*handle, 1) };
                if ok == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            }
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
        match &mut self.kind {
            ChildKind::Std(child) => Ok(child.try_wait()?.and_then(|status| status.code())),
            ChildKind::Raw(handle) => {
                // SAFETY: The handle is a valid process handle.
                let result = unsafe { WaitForSingleObject(*handle, 0) };
                if result == WAIT_TIMEOUT {
                    Ok(None)
                } else if result == WAIT_OBJECT_0 {
                    let mut code: u32 = 0;
                    // SAFETY: The process handle is valid and `code` is writable.
                    let ok = unsafe { GetExitCodeProcess(*handle, &mut code) };
                    if ok == 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(Some(code as i32))
                    }
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        }
    }
}

impl Drop for RunningChild {
    fn drop(&mut self) {
        if let ChildKind::Raw(handle) = &self.kind {
            // SAFETY: The handle comes from `CreateProcessW` and is closed once.
            unsafe {
                CloseHandle(*handle);
            }
        }
    }
}

/// Start a child with a direct `CreateProcessW` call.
///
/// This path is necessary for `CREATE_NEW_CONSOLE` with a hidden window. The
/// standard library cannot set `STARTUPINFOW.wShowWindow`.
pub fn spawn_raw(
    program: &Path,
    args: &[String],
    creation_flags: u32,
    stdout: &File,
    stderr: &File,
) -> io::Result<RunningChild> {
    let program_text = program.to_string_lossy();
    let mut command_line = quote_arg(&program_text);
    for arg in args {
        command_line.push(' ');
        command_line.push_str(&quote_arg(arg));
    }
    let program_wide = to_wide(&program_text);
    let mut command_wide = to_wide(&command_line);

    let null_input = File::open("NUL")?;
    make_inheritable(&null_input)?;
    make_inheritable(stdout)?;
    make_inheritable(stderr)?;

    // SAFETY: All pointers point to living buffers for the duration of the
    // call. `command_wide` is mutable for the duration of the call.
    unsafe {
        let mut startup: STARTUPINFOW = zeroed();
        startup.cb = size_of::<STARTUPINFOW>() as u32;
        startup.dwFlags = STARTF_USESHOWWINDOW | STARTF_USESTDHANDLES;
        startup.wShowWindow = SW_HIDE as u16;
        startup.hStdInput = null_input.as_raw_handle();
        startup.hStdOutput = stdout.as_raw_handle();
        startup.hStdError = stderr.as_raw_handle();

        let mut info: PROCESS_INFORMATION = zeroed();
        let ok = CreateProcessW(
            program_wide.as_ptr(),
            command_wide.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            creation_flags,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut info,
        );
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        CloseHandle(info.hThread);
        Ok(RunningChild {
            kind: ChildKind::Raw(info.hProcess),
            pid: info.dwProcessId,
        })
    }
}

fn make_inheritable(file: &File) -> io::Result<()> {
    // SAFETY: The handle is valid. The flags set the inherit bit.
    let ok = unsafe {
        SetHandleInformation(
            file.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Quote one argument for `CreateProcessW`, in the `CommandLineToArgvW` form.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                out.push('\\');
            }
            '"' => {
                for _ in 0..=backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push('"');
            }
            _ => {
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    for _ in 0..backslashes {
        out.push('\\');
    }
    out.push('"');
    out
}
