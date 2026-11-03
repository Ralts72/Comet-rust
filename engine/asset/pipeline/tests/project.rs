use anyhow::Result;
use comet_asset_pipeline::{SourceMeta, create_scene, prepare};
use comet_core::{AssetId, ProjectPaths};
use std::{fs, path::Path};

#[test]
fn new_scenes_reuse_reserved_identity_and_reject_existing_or_outside_targets() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("assets/关卡"))?;
    let paths = ProjectPaths::open(&project)?;
    let meta = SourceMeta {
        version: 1,
        id: AssetId::new(),
    };
    let metadata = project.join("assets/关卡/新场景.scene.meta");
    let reserved = serde_json::to_vec(&meta)?;
    fs::write(&metadata, &reserved)?;
    let scene = create_scene(&paths, Path::new("关卡/新场景.scene"))?;
    assert_eq!(fs::read(&metadata)?, reserved);
    let saved = fs::read(&scene)?;
    assert!(create_scene(&paths, Path::new("关卡/新场景.scene")).is_err());
    assert_eq!(fs::read(&scene)?, saved);
    for path in ["../outside.scene", "bad.json", "关卡"] {
        assert!(create_scene(&paths, Path::new(path)).is_err());
    }
    let products = temp.path().join("products");
    prepare(&project, &products)?;
    let store = comet_asset::AssetStore::open(products)?;
    assert!(store.index().products.contains_key(&meta.id));
    #[cfg(unix)]
    {
        let outside = temp.path().join("outside");
        fs::create_dir(&outside)?;
        std::os::unix::fs::symlink(&outside, paths.assets().join("link"))?;
        assert!(create_scene(&paths, Path::new("link/outside.scene")).is_err());
        assert!(!outside.join("outside.scene").exists());
    }
    Ok(())
}
