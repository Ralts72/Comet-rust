use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_asset_pipeline::prepare;
use comet_core::{AssetId, EntityId};
use comet_editor_core::Document;
use comet_scene::{
    ContentSource, FieldOverride,
    schema::{self, FieldValue},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    output: PathBuf,
}
impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        let output = temp.path().join("products");
        fs::create_dir_all(project.join("assets"))?;
        for file in fs::read_dir(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/prefab/assets"),
        )? {
            let file = file?;
            fs::copy(file.path(), project.join("assets").join(file.file_name()))?;
        }
        Ok(Self {
            _temp: temp,
            project,
            output,
        })
    }
    fn source(&self, name: &str) -> PathBuf {
        self.project.join("assets").join(name)
    }
    fn prepare(&self) -> Result<comet_asset_pipeline::BuildReport> {
        prepare(&self.project, &self.output)
    }
    fn read(&self, name: &str) -> Result<serde_json::Value> {
        Ok(serde_json::from_slice(&fs::read(self.source(name))?)?)
    }
    fn write(&self, name: &str, value: &serde_json::Value) -> Result<()> {
        fs::write(self.source(name), serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }
}
fn id(n: u32) -> AssetId {
    format!("10000000-0000-4000-8000-{n:012}").parse().unwrap()
}

#[test]
fn products_are_deterministic_and_load_without_authoring_sources() -> Result<()> {
    for translated in [false, true] {
        let f = Fixture::new()?;
        if translated {
            let mut source = f.read("main.scene")?;
            source["version"] = 2.into();
            source["instances"][0]["translation"] = serde_json::json!([10., 2., -3.]);
            f.write("main.scene", &source)?;
        }
        let first = f.prepare()?;
        assert_eq!(first.changed, 6);
        let next = f.prepare()?;
        assert_eq!(next.changed, 0);
        assert_eq!(next.reused, 6);
        assert_eq!(first.index_digest, next.index_digest);
        let mut store = AssetStore::open(&f.output)?;
        assert_eq!(store.resident_count(), 0);
        assert_eq!(store.dependency_closure(id(1))?.len(), 6);
        let scene = store.expand_scene(id(1))?;
        assert_eq!(scene.entities.len(), 7);
        assert_eq!(
            scene.entities.iter().filter(|e| e.model.is_some()).count(),
            2
        );
        for e in scene.entities.iter().filter(|e| e.model.is_some()) {
            assert_eq!(e.model.as_ref().unwrap().surface.roughness, 0.25);
        }
        match &store.load(id(6))?.payload {
            Product::Texture(t) => {
                assert_eq!((t.width, t.height), (2, 1));
                assert_eq!(t.rgba8.len(), 8);
            }
            _ => panic!("wrong texture product"),
        }
        fs::remove_dir_all(&f.project)?;
        assert_eq!(AssetStore::open(&f.output)?.expand_scene(id(1))?, scene);
    }
    Ok(())
}
#[test]
fn legacy_filenames_cannot_drop_products_or_regenerate_metadata_ids() -> Result<()> {
    let f = Fixture::new()?;
    f.prepare()?;
    let index = fs::read(f.output.join("index.json"))?;
    let metadata = fs::read(f.source("main.scene.meta"))?;
    for name in [
        "main.scene",
        "leaf.prefab",
        "mesh.mesh",
        "material.material",
        "main.scene.meta",
    ] {
        let legacy = format!("{name}.json");
        fs::rename(f.source(name), f.source(&legacy))?;
        let error = format!("{:#}", f.prepare().unwrap_err());
        assert!(error.contains("legacy asset filename") && error.contains(&legacy));
        assert_eq!(fs::read(f.output.join("index.json"))?, index);
        assert!(!f.source(name).exists());
        fs::rename(f.source(&legacy), f.source(name))?;
        assert_eq!(f.prepare()?.changed, 0);
        assert_eq!(fs::read(f.source("main.scene.meta"))?, metadata);
    }
    Ok(())
}
#[test]
fn source_update_refresh_and_rename_preserve_identity_and_old_versions() -> Result<()> {
    let f = Fixture::new()?;
    f.prepare()?;
    let mut store = AssetStore::open(&f.output)?;
    let old_leaf = store.load(id(3))?;
    let old_mesh = store.load(id(4))?;
    let before = store.expand_scene(id(1))?;
    let mut leaf = f.read("leaf.prefab")?;
    leaf["base"]["entities"][0]["name"] = "source updated".into();
    leaf["base"]["entities"][0]["model"]["surface"]["roughness"] = 0.9.into();
    f.write("leaf.prefab", &leaf)?;
    assert_eq!(f.prepare()?.changed, 1);
    assert_eq!(
        store.refresh()?.into_iter().collect::<Vec<_>>(),
        vec![id(3)]
    );
    let current = store.load(id(3))?;
    assert!(!Arc::ptr_eq(&old_leaf, &current));
    assert!(Arc::ptr_eq(&old_mesh, &store.load(id(4))?));
    match &old_leaf.payload {
        Product::Prefab(s) => assert_eq!(
            s.base.entities[0].model.as_ref().unwrap().surface.roughness,
            0.6
        ),
        _ => unreachable!(),
    }
    let after = store.expand_scene(id(1))?;
    assert_eq!(
        before.entities.iter().map(|e| e.id).collect::<Vec<_>>(),
        after.entities.iter().map(|e| e.id).collect::<Vec<_>>()
    );
    for e in after.entities.iter().filter(|e| e.model.is_some()) {
        assert_eq!(e.name, "source updated");
        assert_eq!(e.model.as_ref().unwrap().surface.roughness, 0.25);
    }
    let old_index = store.shared_index();
    let metadata = fs::read(f.source("leaf.prefab.meta"))?;
    fs::create_dir_all(f.source("moved/models"))?;
    fs::rename(
        f.source("leaf.prefab"),
        f.source("moved/models/renamed.prefab"),
    )?;
    fs::rename(
        f.source("leaf.prefab.meta"),
        f.source("moved/models/renamed.prefab.meta"),
    )?;
    assert_eq!(f.prepare()?.changed, 0);
    assert!(store.refresh()?.is_empty());
    assert!(!Arc::ptr_eq(&old_index, &store.shared_index()));
    assert!(Arc::ptr_eq(&current, &store.load(id(3))?));
    assert!(Arc::ptr_eq(&old_mesh, &store.load(id(4))?));
    assert_eq!(
        store.index().products[&id(3)]
            .editor
            .as_ref()
            .unwrap()
            .label,
        "moved/models/renamed.prefab"
    );
    assert_eq!(
        fs::read(f.source("moved/models/renamed.prefab.meta"))?,
        metadata
    );
    store.evict(id(3));
    assert_eq!(*store.load(id(3))?, *current);
    Ok(())
}

#[test]
fn prepare_upgrades_v2_cache_without_rewriting_products_or_author_identities() -> Result<()> {
    use comet_asset_data::product::{INDEX_VERSION, PRODUCT_VERSION, ProductIndex};
    let f = Fixture::new()?;
    f.prepare()?;
    let mut store = AssetStore::open(&f.output)?;
    let mut old = store.index().clone();
    let meta = fs::read(f.source("leaf.prefab.meta"))?;
    let product = store.load(id(3))?;
    old.version = 2;
    for entry in old.products.values_mut() {
        entry.editor = None;
    }
    let index_path = f.output.join("index.json");
    fs::write(&index_path, serde_json::to_vec(&old)?)?;
    assert!(AssetStore::open(&f.output).is_err());
    assert_eq!(f.prepare()?.changed, 0);
    let mut rebuilt = AssetStore::open(&f.output)?;
    assert_eq!(rebuilt.index().version, INDEX_VERSION);
    for (id, entry) in &old.products {
        assert!(entry.same_product(&rebuilt.index().products[id]));
        assert!(rebuilt.index().products[id].editor.is_some());
    }
    assert_eq!(rebuilt.load(id(3))?.version, PRODUCT_VERSION);
    assert_eq!(*rebuilt.load(id(3))?, *product);
    assert_eq!(fs::read(f.source("leaf.prefab.meta"))?, meta);
    let mut forged: ProductIndex = rebuilt.index().clone();
    forged.version = 2;
    let bytes = serde_json::to_vec(&forged)?;
    fs::write(&index_path, &bytes)?;
    assert!(f.prepare().is_err());
    assert_eq!(fs::read(index_path)?, bytes);
    let mut with_null = serde_json::to_value(&old)?;
    with_null["products"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["editor"] = serde_json::Value::Null;
    let bytes = serde_json::to_vec(&with_null)?;
    let index_path = f.output.join("index.json");
    fs::write(&index_path, &bytes)?;
    assert!(
        f.prepare().is_err(),
        "legacy schema never had an editor field, even null"
    );
    assert_eq!(fs::read(index_path)?, bytes);
    Ok(())
}
#[test]
fn failed_import_never_publishes_a_partial_index() -> Result<()> {
    let f = Fixture::new()?;
    f.prepare()?;
    let before = fs::read(f.output.join("index.json"))?;
    let mut mesh = f.read("mesh.mesh")?;
    mesh["indices"][2] = 999.into();
    f.write("mesh.mesh", &mesh)?;
    let error = format!("{:#}", f.prepare().unwrap_err());
    assert!(error.contains("mesh.mesh") && error.contains("out of bounds"));
    assert_eq!(fs::read(f.output.join("index.json"))?, before);
    assert_eq!(
        AssetStore::open(&f.output)?
            .expand_scene(id(1))?
            .entities
            .len(),
        7
    );
    Ok(())
}
#[test]
fn missing_dependencies_duplicate_ids_and_prefab_cycles_are_diagnosed() -> Result<()> {
    let f = Fixture::new()?;
    f.prepare()?;
    let before = fs::read(f.output.join("index.json"))?;
    let original = f.read("leaf.prefab")?;
    let mut leaf = original.clone();
    leaf["base"]["entities"][0]["model"]["mesh"] = AssetId::new().to_string().into();
    f.write("leaf.prefab", &leaf)?;
    assert!(format!("{:#}", f.prepare().unwrap_err()).contains("missing dependency"));
    f.write("leaf.prefab", &original)?;
    fs::copy(f.source("leaf.prefab"), f.source("duplicate.prefab"))?;
    fs::copy(
        f.source("leaf.prefab.meta"),
        f.source("duplicate.prefab.meta"),
    )?;
    assert!(format!("{:#}", f.prepare().unwrap_err()).contains("duplicate asset ID"));
    fs::remove_file(f.source("duplicate.prefab"))?;
    fs::remove_file(f.source("duplicate.prefab.meta"))?;
    let mut branch = f.read("branch.prefab")?;
    branch["instances"][0]["prefab"] = id(2).to_string().into();
    branch["instances"][0]["overrides"] = serde_json::json!([]);
    f.write("branch.prefab", &branch)?;
    assert!(format!("{:#}", f.prepare().unwrap_err()).contains("cycle"));
    assert_eq!(fs::read(f.output.join("index.json"))?, before);
    Ok(())
}
#[test]
fn failed_refresh_keeps_the_accepted_store_and_resident_products() -> Result<()> {
    let f = Fixture::new()?;
    f.prepare()?;
    let mut store = AssetStore::open(&f.output)?;
    let old = store.load(id(4))?;
    let index = fs::read(f.output.join("index.json"))?;
    let mut bad: serde_json::Value = serde_json::from_slice(&index)?;
    bad["version"] = 99.into();
    fs::write(f.output.join("index.json"), serde_json::to_vec(&bad)?)?;
    assert!(store.refresh().is_err());
    assert!(Arc::ptr_eq(&old, &store.load(id(4))?));
    fs::write(f.output.join("index.json"), index)?;
    let entry = &store.index().products[&id(4)];
    let file = f
        .output
        .join("products")
        .join(id(4).to_string())
        .join(format!("{}.json", entry.digest));
    fs::write(&file, b"corrupt")?;
    assert!(format!("{:#}", store.refresh().unwrap_err()).contains("digest mismatch"));
    assert!(Arc::ptr_eq(&old, &store.load(id(4))?));
    assert_eq!(f.prepare()?.changed, 0);
    assert!(store.refresh()?.is_empty());
    Ok(())
}
#[test]
fn new_sources_get_persistent_metadata_and_removal_updates_the_index() -> Result<()> {
    let f = Fixture::new()?;
    f.prepare()?;
    fs::copy(f.source("material.material"), f.source("unused.material"))?;
    assert_eq!(f.prepare()?.changed, 1);
    let meta = f.read("unused.material.meta")?;
    let added: AssetId = meta["id"].as_str().unwrap().parse()?;
    assert_eq!(f.prepare()?.changed, 0);
    let mut store = AssetStore::open(&f.output)?;
    let resident = store.load(added)?;
    fs::remove_file(f.source("unused.material"))?;
    assert_eq!(f.prepare()?.removed, 1);
    assert!(store.refresh()?.contains(&added));
    assert!(store.load(added).is_err());
    assert_eq!(resident.id, added);
    fs::remove_file(f.source("mesh.mesh"))?;
    assert!(f.prepare().is_err());
    Ok(())
}
#[test]
fn document_preserves_prefab_declarations_overrides_and_cloned_attachments() -> Result<()> {
    let f = Fixture::new()?;
    f.prepare()?;
    let source = ContentSource::from_json(&fs::read(f.source("main.scene"))?)?;
    let root = source.base.entities[0].id;
    let first = source.instances[0].id;
    let mut doc = Document::open_source(source.clone())?;
    let mut store = AssetStore::open(&f.output)?;
    let entity: EntityId = "20000000-0000-4000-8000-000000000003".parse()?;
    doc.begin()?;
    doc.preview_override(
        first,
        FieldOverride {
            entity,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([5., 0., 0.]),
        },
        &mut store,
    )?;
    doc.commit()?;
    assert_eq!(doc.source().instances.len(), 2);
    doc.undo()?;
    assert_eq!(doc.source(), source);
    doc.redo()?;
    let before = doc.source();
    doc.begin()?;
    assert!(
        doc.preview_override(
            first,
            FieldOverride {
                entity,
                field: schema::ROUGHNESS,
                value: FieldValue::Scalar(0.1)
            },
            &mut store
        )
        .is_err()
    );
    assert_eq!(doc.source(), before);
    doc.cancel()?;
    let copies = doc.clone_subtrees(&[root], &mut store)?;
    assert_eq!(doc.source().instances.len(), 4);
    assert_eq!(
        doc.source()
            .instances
            .iter()
            .filter(|i| i.parent == Some(copies[&root]))
            .count(),
        2
    );
    let path = f.source("edited.scene");
    doc.save(&path)?;
    assert_eq!(Document::load(&path)?.source(), doc.source());
    assert_eq!(doc.source().expand(&mut store)?.entities.len(), 14);
    Ok(())
}
#[test]
fn invalid_png_and_concurrent_build_are_rejected_before_publication() -> Result<()> {
    let f = Fixture::new()?;
    fs::write(f.source("texture.png"), b"bad PNG")?;
    assert!(format!("{:#}", f.prepare().unwrap_err()).contains("texture.png"));
    assert!(!f.output.join("index.json").exists());
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.output.join(".build.lock"))?;
    lock.try_lock()?;
    assert!(format!("{:#}", f.prepare().unwrap_err()).contains("output lock"));
    Ok(())
}
