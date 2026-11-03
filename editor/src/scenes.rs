use anyhow::{Result, ensure};
use comet_asset::AssetStore;
use comet_core::{AssetId, ProjectPaths};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct SceneChoice {
    pub id: AssetId,
    pub label: String,
    pub source: PathBuf,
}
pub(crate) fn collect(paths: &ProjectPaths, assets: &AssetStore) -> Result<Vec<SceneChoice>> {
    let mut choices = vec![];
    collect_scenes(paths.assets(), paths, assets, &mut choices)?;
    choices.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(choices)
}
fn collect_scenes(
    directory: &Path,
    paths: &ProjectPaths,
    assets: &AssetStore,
    choices: &mut Vec<SceneChoice>,
) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(!kind.is_symlink(), "source symlinks are not supported");
        if kind.is_dir() {
            collect_scenes(&entry.path(), paths, assets, choices)?;
        } else if entry.path().extension().is_some_and(|e| e == "scene") {
            let path = paths.resolve_asset_path(entry.path())?;
            let meta: comet_asset_pipeline::SourceMeta =
                serde_json::from_slice(&std::fs::read(format!("{}.meta", path.display()))?)?;
            ensure!(
                assets
                    .index()
                    .products
                    .get(&meta.id)
                    .is_some_and(|p| p.kind == comet_asset_data::product::AssetKind::Scene),
                "scene metadata does not match the product index"
            );
            choices.push(SceneChoice {
                id: meta.id,
                source: path.strip_prefix(paths.assets())?.to_owned(),
                label: path
                    .strip_prefix(paths.assets())?
                    .to_string_lossy()
                    .into_owned(),
            });
        }
    }
    Ok(())
}
