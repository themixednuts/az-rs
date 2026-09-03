use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::Duration;

use crate::manifest::ProjectManifestError;

const ATOMIC_REPLACE_ATTEMPTS: usize = 8;
#[cfg(windows)]
const ATOMIC_REPLACE_RETRY_DELAY: Duration = Duration::from_millis(10);

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Durably replace one generated file without exposing a partial write.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), ProjectManifestError> {
    let parent = path.parent().ok_or_else(|| ProjectManifestError::Write {
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic-write destination has no parent directory",
        ),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| ProjectManifestError::Write {
        path: parent.to_path_buf(),
        source,
    })?;

    let (temporary_path, mut temporary) = create_atomic_temp_file(path)?;
    let result = (|| {
        temporary
            .write_all(contents)
            .and_then(|()| temporary.sync_all())
            .map_err(|source| ProjectManifestError::Write {
                path: temporary_path.clone(),
                source,
            })?;
        drop(temporary);
        atomic_replace(&temporary_path, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

fn create_atomic_temp_file(path: &Path) -> Result<(PathBuf, File), ProjectManifestError> {
    for _ in 0..ATOMIC_REPLACE_ATTEMPTS {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_path =
            path.with_extension(format!("azoth-{}-{sequence}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(ProjectManifestError::Write {
                    path: temporary_path,
                    source,
                });
            }
        }
    }
    Err(ProjectManifestError::Write {
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique atomic-write temporary file",
        ),
    })
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), ProjectManifestError> {
    std::fs::rename(source, destination).map_err(|source| ProjectManifestError::Write {
        path: destination.to_path_buf(),
        source,
    })
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), ProjectManifestError> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut last_error = None;
    for attempt in 0..ATOMIC_REPLACE_ATTEMPTS {
        // SAFETY: both buffers are owned, NUL-terminated UTF-16 paths and stay
        // alive for the duration of the Win32 call.
        let result = unsafe {
            MoveFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(destination_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < ATOMIC_REPLACE_ATTEMPTS {
            std::thread::sleep(ATOMIC_REPLACE_RETRY_DELAY);
        }
    }
    Err(ProjectManifestError::Write {
        path: destination.to_path_buf(),
        source: std::io::Error::other(last_error.map_or_else(
            || "atomic replacement failed".to_string(),
            |error| error.to_string(),
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_complete_file_without_temp_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snapshot.json");
        std::fs::write(&path, "old").unwrap();

        atomic_write(&path, b"new complete contents").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new complete contents");
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            1,
            "failed atomic writes must not leave temporary files"
        );
    }
}
