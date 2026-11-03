//! Win32 resource updates apply only to the unpublished executable copy.
use anyhow::{Context, Result, ensure};
use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};
type Handle = *mut c_void;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn FreeLibrary(module: Handle) -> i32;
    fn EnumResourceLanguagesW(
        module: Handle,
        kind: *const u16,
        name: *const u16,
        callback: unsafe extern "system" fn(Handle, *const u16, *const u16, u16, isize) -> i32,
        context: isize,
    ) -> i32;
    fn BeginUpdateResourceW(path: *const u16, delete: i32) -> Handle;
    fn UpdateResourceW(
        update: Handle,
        kind: *const u16,
        name: *const u16,
        language: u16,
        data: *const c_void,
        size: u32,
    ) -> i32;
    fn EndUpdateResourceW(update: Handle, discard: i32) -> i32;
    fn FindResourceExW(module: Handle, kind: *const u16, name: *const u16, language: u16)
    -> Handle;
    fn SizeofResource(module: Handle, resource: Handle) -> u32;
    fn LoadResource(module: Handle, resource: Handle) -> Handle;
    fn LockResource(resource: Handle) -> *const c_void;
}
struct Module(Handle);
impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: the non-null module is owned here and no resource borrows outlive it.
        unsafe {
            FreeLibrary(self.0);
        }
    }
}
struct Update(Handle);
impl Drop for Update {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: an uncommitted handle is owned here; discard cannot publish a partial update.
            unsafe {
                EndUpdateResourceW(self.0, 1);
            }
        }
    }
}
fn check(result: i32) -> Result<()> {
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
unsafe extern "system" fn language(
    _: Handle,
    _: *const u16,
    _: *const u16,
    id: u16,
    context: isize,
) -> i32 {
    // SAFETY: EnumResourceLanguagesW invokes synchronously with our live, exclusively borrowed Vec.
    unsafe {
        (*(context as *mut Vec<u16>)).push(id);
    }
    1
}
fn load(path: &[u16]) -> Result<Module> {
    // SAFETY: path is NUL terminated and lives through the call; data-only mapping executes no code.
    let module = unsafe { LoadLibraryExW(path.as_ptr(), ptr::null_mut(), 0x2 | 0x20) };
    ensure!(
        !module.is_null(),
        "无法读取 Windows 图标资源：{}",
        std::io::Error::last_os_error()
    );
    Ok(Module(module))
}
pub(super) fn embed(binary: &Path, images: &[(u16, Vec<u8>)]) -> Result<()> {
    let mut permissions = std::fs::metadata(binary)?.permissions();
    #[expect(
        clippy::permissions_set_readonly_false,
        reason = "Windows-only: clears the read-only file attribute; Unix permissions are unaffected"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(binary, permissions)?;
    let path: Vec<u16> = binary
        .canonicalize()?
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut languages = vec![0u16];
    {
        let module = load(&path)?;
        // SAFETY: numeric resource IDs use MAKEINTRESOURCE convention; callback/context stay alive.
        let result = unsafe {
            EnumResourceLanguagesW(
                module.0,
                14 as _,
                1 as _,
                language,
                &mut languages as *mut Vec<u16> as isize,
            )
        };
        if result == 0 {
            let error = std::io::Error::last_os_error();
            ensure!(matches!(error.raw_os_error(), Some(1812..=1815)), "{error}");
        }
    }
    languages.sort_unstable();
    languages.dedup();
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut resources = vec![];
    for (index, (size, image)) in images.iter().enumerate() {
        let id = index as u16 + 1;
        group.extend_from_slice(&[*size as u8, *size as u8, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(image.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
        resources.push((3u16, id, image.as_slice()));
    }
    resources.push((14, 1, group.as_slice()));
    // SAFETY: the private candidate isn't running or mapped; path is NUL terminated.
    let mut update = Update(unsafe { BeginUpdateResourceW(path.as_ptr(), 0) });
    ensure!(!update.0.is_null(), "{}", std::io::Error::last_os_error());
    for language in &languages {
        for (kind, id, bytes) in &resources {
            // SAFETY: all resource buffers stay alive until the update is committed below.
            check(unsafe {
                UpdateResourceW(
                    update.0,
                    *kind as usize as _,
                    *id as usize as _,
                    *language,
                    bytes.as_ptr().cast(),
                    bytes.len().try_into()?,
                )
            })
            .context("写入游戏图标失败")?;
        }
    }
    let handle = std::mem::replace(&mut update.0, ptr::null_mut());
    // SAFETY: this owned update handle is consumed exactly once by EndUpdateResourceW.
    check(unsafe { EndUpdateResourceW(handle, 0) })?;
    let module = load(&path)?;
    for language in languages {
        for (kind, id, expected) in &resources {
            // SAFETY: module remains mapped while resource pointers are read; IDs are numeric.
            unsafe {
                let resource =
                    FindResourceExW(module.0, *kind as usize as _, *id as usize as _, language);
                ensure!(!resource.is_null(), "缺少导出游戏图标资源");
                let size = SizeofResource(module.0, resource) as usize;
                let loaded = LoadResource(module.0, resource);
                ensure!(!loaded.is_null(), "无法载入导出游戏图标资源");
                let data = LockResource(loaded);
                ensure!(
                    !data.is_null() && size == expected.len(),
                    "导出游戏图标资源尺寸不匹配"
                );
                ensure!(
                    std::slice::from_raw_parts(data.cast::<u8>(), size) == *expected,
                    "导出游戏图标校验失败"
                );
            }
        }
    }
    Ok(())
}
