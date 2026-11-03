use anyhow::{Context, Result, ensure};
use comet_core::{AssetId, ProjectPaths, atomic_write};
use serde::{Deserialize, Serialize};

/// Publish metadata before a new scene. An interrupted creation can reuse its reserved identity.
pub fn create_scene(
    paths: &ProjectPaths,
    relative: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let _lock = crate::build::lock(&paths.root().join(".assetc.lock"), "project")?;
    let path = paths.resolve_asset_path(relative)?;
    ensure!(
        path.extension().is_some_and(|e| e == "scene"),
        "场景文件必须使用 .scene 后缀"
    );
    ensure!(
        std::fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "场景位置已存在"
    );
    let metadata = paths.resolve_asset_path(crate::SourceMeta::path(&path))?;
    match std::fs::read(&metadata) {
        Ok(bytes) => {
            crate::SourceMeta::from_bytes(&bytes).context("读取保留的场景身份失败")?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let meta = crate::SourceMeta {
                version: 1,
                id: AssetId::new(),
            };
            comet_core::atomic_create(&metadata, &serde_json::to_vec_pretty(&meta)?)?;
        }
        Err(e) => return Err(e).context("读取场景身份失败"),
    }
    let path = paths.resolve_asset_path(relative)?;
    comet_core::atomic_create(&path, &comet_scene::ContentSource::default().to_json()?)?;
    Ok(path)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSettings {
    pub version: u32,
    pub game_icon: Option<AssetId>,
}
impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            version: 1,
            game_icon: None,
        }
    }
}
impl ProjectSettings {
    pub fn load(paths: &ProjectPaths) -> Result<Self> {
        reject_symlink(paths)?;
        let path = comet_core::resolve_destination(&paths.root().join("project.json"))?;
        ensure!(
            path.parent() == Some(paths.root()),
            "项目设置不能指向项目外部"
        );
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).context("读取项目设置失败"),
        };
        let settings: Self = serde_json::from_slice(&bytes).context("解析项目设置失败")?;
        ensure!(settings.version == 1, "不支持的项目设置版本");
        Ok(settings)
    }
    pub fn save(&self, paths: &ProjectPaths, assets: &mut comet_asset::AssetStore) -> Result<()> {
        reject_symlink(paths)?;
        ensure!(self.version == 1, "不支持的项目设置版本");
        if let Some(id) = self.game_icon {
            crate::icon::validate(assets, id)?;
        }
        let destination = comet_core::resolve_destination(&paths.root().join("project.json"))?;
        ensure!(
            destination.parent() == Some(paths.root()),
            "项目设置不能指向项目外部"
        );
        atomic_write(&destination, &serde_json::to_vec_pretty(self)?)
    }
}
fn reject_symlink(paths: &ProjectPaths) -> Result<()> {
    match std::fs::symlink_metadata(paths.root().join("project.json")) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink(),
            "项目设置不能使用符号链接"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("检查项目设置路径失败"),
    }
    Ok(())
}
