//! A directory becomes visible at its final name only after the caller validates the candidate.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub struct DirectoryCandidate {
    temporary: PathBuf,
    destination: PathBuf,
    published: bool,
}
impl DirectoryCandidate {
    pub fn new(destination: &Path) -> Result<Self> {
        let destination = crate::resolve_destination(destination)?;
        ensure!(
            fs::symlink_metadata(&destination)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "destination already exists or is inaccessible: {}",
            destination.display()
        );
        let parent = destination
            .parent()
            .context("directory destination has no parent")?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".comet-{}.tmp", Uuid::new_v4()));
        fs::create_dir(&temporary)?;
        Ok(Self {
            temporary,
            destination,
            published: false,
        })
    }
    pub fn path(&self) -> &Path {
        &self.temporary
    }
    pub fn publish(mut self) -> Result<PathBuf> {
        rename_exclusive(&self.temporary, &self.destination)
            .with_context(|| format!("publish directory {}", self.destination.display()))?;
        self.published = true;
        Ok(self.destination.clone())
    }
}
impl Drop for DirectoryCandidate {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.temporary);
        }
    }
}
#[cfg(unix)]
fn rename_exclusive(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    #[cfg(target_os = "linux")]
    // SAFETY: both NUL-terminated paths live for the call; AT_FDCWD uses absolute paths.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    // SAFETY: both NUL-terminated paths live for the call. RENAME_EXCL prevents replacement.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "exclusive directory publication is unsupported on this platform",
    ));
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
#[cfg(windows)]
fn rename_exclusive(source: &Path, destination: &Path) -> std::io::Result<()> {
    // Windows does not replace an existing directory through std::fs::rename.
    fs::rename(source, destination)
}
