//! Shared authoring preflight borrows content; callers own tasks, documents and publication.
use anyhow::{Context, Result};
use comet_asset::AssetStore;
use comet_core::AssetId;
use comet_scene::{ContentSource, SceneSnapshot};

/// Retained undo/redo and savepoint content must remain usable after an asset switch.
pub(crate) fn validate_sources<'a>(
    retained: impl IntoIterator<Item = &'a ContentSource>,
    assets: &mut AssetStore,
    prefab_asset: Option<AssetId>,
) -> Result<SceneSnapshot> {
    let mut current = None;
    let mut scripts = comet_runtime::ScriptValidator::default();
    for (index, source) in retained.into_iter().enumerate() {
        let snapshot = match prefab_asset {
            Some(root) => source.expand_prefab(root, assets),
            None => source.expand(assets),
        }
        .context("文档或历史中的 Prefab 无法展开")?;
        // Script preparation checks every asset reference, including hidden geometry and external links.
        let sources = assets
            .script_sources(&snapshot)
            .context("文档或历史缺少依赖产品，或产品类型不兼容")?;
        validate_authoring(&snapshot, assets).context("文档或历史的绘制或动画配置无效")?;
        scripts
            .validate(&snapshot, &sources)
            .context("文档或历史中的脚本不兼容")?;
        if index == 0 {
            current = Some(snapshot);
        }
    }
    current.context("缺少作者文档")
}

/// Incomplete explicit bindings remain editable; every other asset or draw error still rejects.
pub(crate) fn validate_authoring(snapshot: &SceneSnapshot, assets: &mut AssetStore) -> Result<()> {
    allow_unbound(comet_application::extract(snapshot, assets, &[]).map(|_| ()))?;
    let libraries = assets.animation_sources(snapshot)?;
    allow_unbound(comet_runtime::validate_animations(
        snapshot,
        &libraries.skeletons,
        &libraries.clips,
        &libraries.controllers,
    ))
}

fn allow_unbound(result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.is::<comet_scene::UnboundRig>() => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
