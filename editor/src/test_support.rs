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
    let (editor, assets) = open(project, temp.path().join("products"), "main.scene".into())?;
    Ok((temp, editor, assets))
}
