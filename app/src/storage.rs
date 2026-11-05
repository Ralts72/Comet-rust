use crate::settings::{MAX_SETTINGS_BYTES, Settings};
use anyhow::{Context, Result, ensure};
use comet_core::{Checkpoint, GameId, MAX_CHECKPOINT_BYTES, atomic_write};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

enum Backing {
    Memory {
        checkpoint: Option<Vec<u8>>,
        settings: Option<Vec<u8>>,
    },
    Directory(PathBuf),
}

#[derive(Clone, Copy)]
enum Slot {
    Checkpoint,
    Settings,
}
impl Slot {
    fn name(self) -> &'static str {
        match self {
            Self::Checkpoint => "checkpoint.json",
            Self::Settings => "settings.json",
        }
    }
    fn max_bytes(self) -> usize {
        match self {
            Self::Checkpoint => MAX_CHECKPOINT_BYTES,
            Self::Settings => MAX_SETTINGS_BYTES,
        }
    }
}

/// Two fixed game-owned files; runtime admission and effective preferences belong to App.
pub(super) struct Store {
    game_id: GameId,
    backing: Backing,
}
impl Store {
    pub(super) fn memory(game_id: GameId) -> Self {
        Self {
            game_id,
            backing: Backing::Memory {
                checkpoint: None,
                settings: None,
            },
        }
    }

    /// The explicit root replaces the platform base, not the game UUID directory.
    pub(super) fn directory(game_id: GameId, root: PathBuf) -> Result<Self> {
        ensure!(root.is_absolute(), "游戏数据目录必须是绝对路径");
        Ok(Self {
            game_id,
            backing: Backing::Directory(root),
        })
    }

    pub(super) fn for_user(game_id: GameId) -> Result<Self> {
        Self::directory(game_id, user_directory()?)
    }

    pub(super) fn game_id(&self) -> GameId {
        self.game_id
    }

    pub(super) fn is_persistent(&self) -> bool {
        matches!(self.backing, Backing::Directory(_))
    }

    pub(super) fn load_checkpoint(&self) -> Result<Option<Checkpoint>> {
        self.read(Slot::Checkpoint)?
            .map(|bytes| {
                let checkpoint = Checkpoint::from_bytes(&bytes)?;
                checkpoint.validate_for(self.game_id)?;
                Ok(checkpoint)
            })
            .transpose()
    }

    pub(super) fn save_checkpoint(&mut self, checkpoint: &Checkpoint) -> Result<()> {
        checkpoint.validate_for(self.game_id)?;
        self.write(Slot::Checkpoint, checkpoint.to_bytes()?)
    }

    pub(super) fn load_settings(&self) -> Result<Option<Settings>> {
        self.read(Slot::Settings)?
            .map(|bytes| Settings::from_bytes(&bytes, self.game_id))
            .transpose()
    }

    pub(super) fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        let bytes = settings.to_bytes(self.game_id)?;
        // Rejected existing settings require repair/removal, never an implicit defaults overwrite.
        self.load_settings()
            .context("原游戏设置无效，保留原文件，请修复或移走后重试")?;
        self.write(Slot::Settings, bytes)
    }

    fn read(&self, slot: Slot) -> Result<Option<Vec<u8>>> {
        match &self.backing {
            Backing::Memory {
                checkpoint,
                settings,
            } => Ok(match slot {
                Slot::Checkpoint => checkpoint,
                Slot::Settings => settings,
            }
            .clone()),
            Backing::Directory(root) => {
                let Some(path) = slot_path(root, self.game_id, slot, false)? else {
                    return Ok(None);
                };
                let file = match File::open(&path) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error).context("读取游戏数据失败"),
                };
                let metadata = file.metadata()?;
                ensure!(metadata.is_file(), "游戏数据不是普通文件");
                ensure!(
                    metadata.len() <= slot.max_bytes() as u64,
                    "{} 超过大小限制",
                    slot.name()
                );
                let mut bytes = Vec::new();
                file.take(slot.max_bytes() as u64 + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= slot.max_bytes(),
                    "{} 超过大小限制",
                    slot.name()
                );
                // Recheck the published path after reading, before consuming external data.
                slot_path(root, self.game_id, slot, false)?.context("读取时游戏数据目录已改变")?;
                Ok(Some(bytes))
            }
        }
    }

    fn write(&mut self, slot: Slot, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            bytes.len() <= slot.max_bytes(),
            "{} 超过大小限制",
            slot.name()
        );
        match &mut self.backing {
            Backing::Memory {
                checkpoint,
                settings,
            } => {
                *match slot {
                    Slot::Checkpoint => checkpoint,
                    Slot::Settings => settings,
                } = Some(bytes);
            }
            Backing::Directory(root) => {
                let path =
                    slot_path(root, self.game_id, slot, true)?.context("无法创建游戏数据目录")?;
                atomic_write(&path, &bytes).context("保存游戏数据失败")?;
            }
        }
        Ok(())
    }
}

fn slot_path(root: &Path, game_id: GameId, slot: Slot, create: bool) -> Result<Option<PathBuf>> {
    if create {
        fs::create_dir_all(root).context("创建游戏数据根目录失败")?;
    }
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("解析游戏数据根目录失败"),
    };
    ensure!(root.is_dir(), "游戏数据根目录不是目录");
    let directory = root.join(game_id.to_string());
    if create {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("创建游戏数据目录失败"),
        }
    }
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("检查游戏数据目录失败"),
    };
    ensure!(
        !metadata.file_type().is_symlink() && metadata.is_dir(),
        "游戏数据目录必须是真实目录"
    );
    ensure!(
        directory.canonicalize()? == directory,
        "游戏数据目录超出根目录"
    );
    let path = directory.join(slot.name());
    match fs::symlink_metadata(&path) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink() && metadata.is_file(),
            "游戏数据必须是普通文件，不能是符号链接"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("检查游戏数据文件失败"),
    }
    Ok(Some(path))
}

#[cfg(target_os = "macos")]
fn user_directory() -> Result<PathBuf> {
    use objc2_foundation::{NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};
    use std::{ffi::CStr, os::unix::ffi::OsStrExt};
    objc2::rc::autoreleasepool(|_| {
        let urls = NSFileManager::defaultManager().URLsForDirectory_inDomains(
            NSSearchPathDirectory::ApplicationSupportDirectory,
            NSSearchPathDomainMask::UserDomainMask,
        );
        let url = urls.firstObject().context("系统未提供用户应用数据目录")?;
        ensure!(url.isFileURL(), "系统应用数据目录不是文件路径");
        // NSURL owns this NUL-terminated representation until `url` is released.
        let bytes = unsafe { CStr::from_ptr(url.fileSystemRepresentation().as_ptr()) }.to_bytes();
        let path = Path::new(std::ffi::OsStr::from_bytes(bytes)).join("Comet/games");
        ensure!(path.is_absolute(), "系统应用数据目录不是绝对路径");
        Ok(path)
    })
}

#[cfg(target_os = "windows")]
fn user_directory() -> Result<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_SavedGames, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath},
    };
    let mut buffer = ptr::null_mut();
    // The function initializes an owned, NUL-terminated UTF-16 buffer; the null
    // token selects this user, and CoTaskMemFree releases it on every result.
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_SavedGames,
            KF_FLAG_DONT_VERIFY as u32,
            ptr::null_mut(),
            &mut buffer,
        )
    };
    let path = if result >= 0 && !buffer.is_null() {
        let mut length = 0;
        // Success guarantees a NUL-terminated allocation alive until CoTaskMemFree.
        unsafe {
            while *buffer.add(length) != 0 {
                length += 1;
            }
            Some(PathBuf::from(OsString::from_wide(
                std::slice::from_raw_parts(buffer, length),
            )))
        }
    } else {
        None
    };
    // The shell may allocate a buffer even on failure; null is accepted by free.
    unsafe { CoTaskMemFree(buffer.cast()) };
    let path = path.context("系统未提供用户游戏存档目录")?.join("Comet");
    ensure!(path.is_absolute(), "系统游戏存档目录不是绝对路径");
    Ok(path)
}

#[cfg(target_os = "linux")]
fn user_directory() -> Result<PathBuf> {
    linux_directory(|key| std::env::var_os(key))
}

#[cfg(any(target_os = "linux", test))]
fn linux_directory(
    mut environment: impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    if let Some(path) = environment("XDG_DATA_HOME").map(PathBuf::from)
        && path.is_absolute()
    {
        return Ok(path.join("comet/games"));
    }
    let path = environment("HOME")
        .map(PathBuf::from)
        .context("缺少用户主目录")?;
    ensure!(path.is_absolute(), "用户主目录不是绝对路径");
    Ok(path.join(".local/share/comet/games"))
}

#[cfg(test)]
mod tests;
