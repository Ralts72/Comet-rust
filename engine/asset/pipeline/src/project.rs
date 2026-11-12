use anyhow::{Context, Result, ensure};
use comet_core::{AssetId, DirectoryCandidate, GameId, ProjectPaths, atomic_create, atomic_write};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const DEFAULT_SCENE: &str = "main.scene";

/// Publish a complete authoring project only after its initial products can be loaded.
pub fn create_project(destination: &Path) -> Result<ProjectPaths> {
    let candidate = prepared_project(destination)?;
    ProjectPaths::open(candidate.publish()?)
}

fn prepared_project(destination: &Path) -> Result<DirectoryCandidate> {
    ensure!(
        fs::symlink_metadata(destination)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "项目位置已存在或不可访问：{}",
        destination.display()
    );
    let candidate = DirectoryCandidate::new(destination)?;
    fs::create_dir(candidate.path().join("assets"))?;
    let paths = ProjectPaths::open(candidate.path())?;
    let scene = create_scene(&paths, Path::new(DEFAULT_SCENE))?;
    atomic_create(
        &paths.root().join("project.json"),
        &serde_json::to_vec_pretty(&ProjectSettings::for_new_project())?,
    )?;
    ProjectSettings::load(&paths)?;

    // An unpublished sibling owns disposable validation products; they never enter authoring content.
    let products = DirectoryCandidate::new(&candidate.path().with_extension("validation"))?;
    crate::prepare(paths.root(), products.path()).context("验证新项目资产构建失败")?;
    let mut assets =
        comet_asset::AssetStore::open(products.path()).context("验证新项目产品失败")?;
    assets.expand_scene(crate::SourceMeta::load(&scene)?.id)?;
    // Both create_scene and prepare have released their locks. Later builds create a fresh lock file.
    fs::remove_file(paths.root().join(".assetc.lock")).context("清理项目验证锁文件失败")?;
    Ok(candidate)
}

pub fn create_scene(paths: &ProjectPaths, relative: &Path) -> Result<PathBuf> {
    create_content(paths, relative, "scene", |_| {
        comet_scene::ContentSource::default().to_json()
    })
}

pub fn create_prefab(paths: &ProjectPaths, relative: &Path) -> Result<PathBuf> {
    create_content(paths, relative, "prefab", |_| {
        comet_scene::ContentSource::default().to_json()
    })
}

/// Create a new prefab from validated author content; the existing destination is never replaced.
pub fn create_prefab_with_content(
    paths: &ProjectPaths,
    relative: &Path,
    content: &comet_scene::ContentSource,
    assets: &mut comet_asset::AssetStore,
) -> Result<PathBuf> {
    create_content(paths, relative, "prefab", |root| {
        let snapshot = content.expand_prefab(root, assets)?;
        comet_asset_data::product::validate_scene_dependency_kinds(&snapshot, |id| {
            assets.index().products.get(&id).map(|entry| entry.kind)
        })?;
        // The new root replaces any old product at this identity, including a different kind.
        for dependency in content.dependencies() {
            ensure!(
                dependency != root && !assets.dependency_closure(dependency)?.contains(&root),
                "prefab resource dependency cycle at {root}"
            );
        }
        content.to_json()
    })
}

/// Publish metadata first, then the author file; interrupted creation can reuse its reserved identity.
fn create_content(
    paths: &ProjectPaths,
    relative: &Path,
    extension: &str,
    content: impl FnOnce(AssetId) -> Result<Vec<u8>>,
) -> Result<PathBuf> {
    let _lock = crate::build::lock(&paths.root().join(".assetc.lock"), "project")?;
    let path = paths.resolve_asset_path(relative)?;
    ensure!(
        path.extension().is_some_and(|e| e == extension),
        "作者文件必须使用 .{extension} 后缀"
    );
    ensure!(
        std::fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "作者文件位置已存在"
    );
    let metadata = paths.resolve_asset_path(crate::SourceMeta::path(&path))?;
    let (meta, reserve) = match std::fs::read(&metadata) {
        Ok(bytes) => (
            crate::SourceMeta::from_bytes(&bytes).context("读取保留的作者文件身份失败")?,
            false,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            crate::SourceMeta {
                version: 1,
                id: AssetId::new(),
            },
            true,
        ),
        Err(error) => return Err(error).context("读取作者文件身份失败"),
    };
    // Validate with the final root identity before reserving metadata or publishing any author bytes.
    let bytes = content(meta.id)?;
    if reserve {
        meta.reserve(&path)?;
    }
    let current = crate::SourceMeta::load(&path)?;
    ensure!(current.id == meta.id, "作者文件身份在创建期间发生变化");
    let path = paths.resolve_asset_path(relative)?;
    atomic_create(&path, &bytes)?;
    Ok(path)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSettings {
    pub version: u32,
    pub game_icon: Option<AssetId>,
    pub game_id: GameId,
}
impl ProjectSettings {
    /// Allocate identity only when creating a new authoring project.
    pub fn for_new_project() -> Self {
        Self {
            version: 1,
            game_icon: None,
            game_id: GameId::new(),
        }
    }
    pub fn load(paths: &ProjectPaths) -> Result<Self> {
        let path = settings_path(paths)?;
        let file = fs::File::open(&path).context("读取项目设置失败")?;
        let bytes = comet_core::read_bounded(file, 64 * 1024)
            .context("读取项目设置失败（大小限制 64 KiB）")?;
        let settings: Self = serde_json::from_slice(&bytes).context("解析项目设置失败")?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn save(&self, paths: &ProjectPaths, assets: &mut comet_asset::AssetStore) -> Result<()> {
        self.validate()?;
        let _lock = crate::build::lock(&paths.root().join(".assetc.lock"), "project")?;
        let previous = Self::load(paths)?;
        ensure!(
            self.game_id == previous.game_id,
            "项目游戏身份已变化，不能替换身份，请重新打开项目"
        );
        if let Some(id) = self.game_icon {
            crate::icon::validate(assets, id)?;
        }
        ensure!(Self::load(paths)? == previous, "项目设置在保存期间发生变化");
        let destination = settings_path(paths)?;
        atomic_write(&destination, &serde_json::to_vec_pretty(self)?)
    }
    fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "不支持的项目设置版本或游戏身份");
        Ok(())
    }
}

fn settings_path(paths: &ProjectPaths) -> Result<PathBuf> {
    match std::fs::symlink_metadata(paths.root().join("project.json")) {
        Ok(metadata) => ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "项目设置必须是常规文件，不能使用符号链接"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("检查项目设置路径失败"),
    }
    let path = comet_core::resolve_destination(&paths.root().join("project.json"))?;
    ensure!(
        path.parent() == Some(paths.root()),
        "项目设置不能指向项目外部"
    );
    Ok(path)
}

#[cfg(test)]
mod tests;
