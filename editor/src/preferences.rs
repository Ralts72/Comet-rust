//! Shared location and file boundary for optional editor preferences.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn directory(
    os: &str,
    mut get: impl FnMut(&str) -> Option<OsString>,
) -> Result<PathBuf> {
    fn base(name: &str, value: Option<OsString>) -> Result<PathBuf> {
        let path = PathBuf::from(
            value.with_context(|| format!("{name} is unavailable for editor state"))?,
        );
        ensure!(
            !path.as_os_str().is_empty() && path.is_absolute(),
            "{name} must be an absolute nonempty editor state path"
        );
        Ok(path)
    }
    Ok(match os {
        "macos" => base("HOME", get("HOME"))?.join("Library/Application Support/Comet/editor"),
        "windows" => base("APPDATA", get("APPDATA"))?.join("Comet/editor"),
        "linux" => match get("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
            Some(value) => base("XDG_STATE_HOME", Some(value))?.join("comet/editor"),
            None => base("HOME", get("HOME"))?.join(".local/state/comet/editor"),
        },
        _ => anyhow::bail!("unsupported editor user-state platform: {os}"),
    })
}
pub(crate) fn check_file(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && path.file_name().is_some(),
        "editor state must be an absolute file path"
    );
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            metadata.file_type().is_file(),
            "editor state must be a regular file: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect editor user state"),
    }
    Ok(())
}
