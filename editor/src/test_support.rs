//! Shared temporary project setup; tests never write into checked-in assets.
use crate::session::{Editor, open};
use anyhow::Result;
use comet_asset::AssetStore;

pub(crate) fn fixture() -> Result<(tempfile::TempDir, Editor, AssetStore)> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    std::fs::create_dir_all(project.join("assets"))?;
    for file in std::fs::read_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/gltf/assets"),
    )? {
        let file = file?;
        if file.file_name() != ".assetc.lock" {
            std::fs::copy(file.path(), project.join("assets").join(file.file_name()))?;
        }
    }
    std::fs::write(
        project.join("project.json"),
        serde_json::to_vec(&comet_asset_pipeline::ProjectSettings::for_new_project())?,
    )?;
    let (editor, assets) = open(project, temp.path().join("products"), "main.scene".into())?;
    Ok((temp, editor, assets))
}

/// Tests share the checked-in source and import only into their own temporary projects.
pub(crate) fn prepare_skin_asset(editor: &Editor) -> Result<comet_core::AssetId> {
    let destination = editor.project_root().join("assets/model");
    std::fs::create_dir(&destination)?;
    let shared =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../demo/assets/models/simple_skin");
    for file in std::fs::read_dir(shared)? {
        let file = file?;
        std::fs::copy(file.path(), destination.join(file.file_name()))?;
    }
    let source = comet_asset_pipeline::SourceMeta::load(&destination.join("SimpleSkin.gltf"))?.id;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    Ok(source)
}
