use anyhow::Result;
use comet_asset::AssetStore;
use comet_scene::{ContentSource, EntityData, PhysicsBody};
use std::fs;

#[test]
fn physics_products_preserve_metadata_and_reject_invalid_rebuilds_before_publication() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let products = temp.path().join("products");
    fs::create_dir_all(project.join("assets"))?;
    let mut entity = EntityData::new("floor");
    entity.physics = Some(PhysicsBody::default());
    let mut source = ContentSource::default();
    source.base.entities.push(entity);
    let path = project.join("assets/main.scene");
    fs::write(&path, source.to_json()?)?;
    comet_asset_pipeline::prepare(&project, &products)?;
    let metadata_path = project.join("assets/main.scene.meta");
    let metadata = fs::read(&metadata_path)?;
    let id: comet_core::AssetId = serde_json::from_slice::<serde_json::Value>(&metadata)?["id"]
        .as_str()
        .unwrap()
        .parse()?;
    let index = fs::read(products.join("index.json"))?;
    let mut store = AssetStore::open(&products)?;
    assert_eq!(store.expand_scene(id)?, source.base);
    source.base.entities[0].physics.as_mut().unwrap().mass = -1.;
    fs::write(&path, serde_json::to_vec(&source)?)?;
    assert!(comet_asset_pipeline::prepare(&project, &products).is_err());
    assert_eq!(fs::read(products.join("index.json"))?, index);
    assert_eq!(fs::read(metadata_path)?, metadata);
    assert_eq!(
        store.expand_scene(id)?.entities[0]
            .physics
            .as_ref()
            .unwrap()
            .mass,
        1.
    );
    Ok(())
}
