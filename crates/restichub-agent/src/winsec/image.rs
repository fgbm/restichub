//! The executable that this process runs from, as the source of the agent copy (threat T20).
//!
//! Windows refuses a write to a running image but allows a rename. A process without elevation
//! can thus rename the package file in a user folder and put another file at its path before the
//! copy reads it. The path that the loader gives does not follow the rename, but the name of the
//! mapped image does.

use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo, GetFileInformationByHandle,
    GetFileInformationByHandleEx,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::ProcessStatus::K32GetMappedFileNameW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// Opens `path` for the copy and refuses it unless it is the image of this process.
///
/// The handle shares only the read, so the file cannot change or move while the copy reads it.
pub(crate) fn open_running_image(path: &Path) -> Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let image = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(mapped_image_name()?)
        .context("cannot open the image of the agent that runs")?;
    if file_key(&file)? != file_key(&image)? {
        bail!(
            "{} is not the file that runs, so another process may have replaced it after the start; the agent does not copy it; put the release package in a folder that only administrators can change and run the command again",
            path.display()
        );
    }
    Ok(file)
}

/// The current NT name of the mapped image, in a form that `CreateFileW` opens.
fn mapped_image_name() -> Result<String> {
    let mut buffer = vec![0_u16; 32 * 1024];
    let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    // SAFETY: a null name asks for the module of the process image. The buffer is a local of
    // `capacity` UTF-16 units.
    let written = unsafe {
        let base = GetModuleHandleW(std::ptr::null());
        K32GetMappedFileNameW(GetCurrentProcess(), base, buffer.as_mut_ptr(), capacity)
    };
    if written == 0 || written >= capacity {
        return Err(std::io::Error::last_os_error())
            .context("cannot read the name of the agent image");
    }
    buffer.truncate(written as usize);
    Ok(format!(
        r"\\?\GLOBALROOT{}",
        String::from_utf16(&buffer).context("the name of the agent image is not UTF-16")?
    ))
}

/// Two keys of different kinds never match, so a mix refuses the copy.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileKey {
    Id { volume: u64, file: [u8; 16] },
    Index { volume: u32, file: u64 },
}

fn file_key(file: &std::fs::File) -> Result<FileKey> {
    key_or_index(|| id_key(file), || index_key(file))
}

/// FAT, exFAT and some SMB servers do not answer `FileIdInfo`. The older file index stays unique
/// on the volume while the file is open, and a rename that changes it only refuses the copy.
fn key_or_index(
    id: impl FnOnce() -> std::io::Result<FileKey>,
    index: impl FnOnce() -> std::io::Result<FileKey>,
) -> Result<FileKey> {
    match id() {
        Err(error)
            if matches!(
                error
                    .raw_os_error()
                    .and_then(|code| u32::try_from(code).ok()),
                Some(ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED)
            ) =>
        {
            index().context("cannot read the file index")
        }
        key => key.context("cannot read the file ID"),
    }
}

fn id_key(file: &std::fs::File) -> std::io::Result<FileKey> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: the handle is open and the buffer is a local of the given size.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<FILE_ID_INFO>()).unwrap_or(u32::MAX),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FileKey::Id {
        volume: info.VolumeSerialNumber,
        file: info.FileId.Identifier,
    })
}

fn index_key(file: &std::fs::File) -> std::io::Result<FileKey> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is open and the buffer is a local.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FileKey::Index {
        volume: info.dwVolumeSerialNumber,
        file: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn fr_in_07_the_agent_copies_only_the_image_that_runs() {
        let running = std::env::current_exe().expect("cannot find the test executable");
        open_running_image(&running).expect("the running image passes");

        // The state after a swap: the start path names a file with the same content.
        let swapped = super::super::tests::temp("swapped-image.exe");
        std::fs::copy(&running, &swapped).expect("cannot copy the image");
        let error = format!(
            "{:#}",
            open_running_image(&swapped).expect_err("another file at the path")
        );
        assert!(error.contains("is not the file that runs"), "{error}");
        let _ = std::fs::remove_file(&swapped);
    }

    #[test]
    fn fr_in_07_a_volume_without_file_ids_compares_the_file_index() {
        let os_error = |code: u32| {
            std::io::Error::from_raw_os_error(i32::try_from(code).expect("a Windows error code"))
        };
        let indexed = FileKey::Index { volume: 7, file: 9 };
        for unsupported in [ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED] {
            let key = key_or_index(|| Err(os_error(unsupported)), || Ok(indexed.clone()))
                .expect("the index answers");
            assert_eq!(key, indexed);
        }
        let denied = key_or_index(
            || {
                Err(os_error(
                    windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED,
                ))
            },
            || panic!("another error must not fall back"),
        )
        .expect_err("access denied");
        assert!(format!("{denied:#}").contains("cannot read the file ID"));

        let running = std::env::current_exe().expect("cannot find the test executable");
        let open = |path: &Path| std::fs::File::open(path).expect("cannot open the file");
        let copy = super::super::tests::temp("indexed-image.exe");
        std::fs::copy(&running, &copy).expect("cannot copy the image");
        let first = index_key(&open(&running)).expect("index");
        assert_eq!(first, index_key(&open(&running)).expect("index"));
        assert_ne!(first, index_key(&open(&copy)).expect("index"));
        let _ = std::fs::remove_file(&copy);
    }
}
