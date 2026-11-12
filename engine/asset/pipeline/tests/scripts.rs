use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::product::{AssetKind, INDEX_VERSION, MAX_SCRIPT_BYTES, Product};
use comet_asset_pipeline::{
    DEFAULT_SCENE, SourceCatalog, SourceMeta, create_project, package, prepare,
};
use comet_core::{AssetId, ProjectPaths, digest};
use comet_scene::{ContentSource, EntityData, SceneSnapshot, Script};
use std::{fs, path::PathBuf};

const SOURCE: &str = "fn update(dt) { let 中文 = dt; }";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    products: PathBuf,
    script: AssetId,
}
impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("project");
        let products = temp.path().join("products");
        create_project(&root)?;
        fs::write(root.join("assets/movement.rhai"), SOURCE)?;
        prepare(&root, &products)?;
        let script = SourceMeta::load(&root.join("assets/movement.rhai"))?.id;
        Ok(Self {
            _temp: temp,
            root,
            products,
            script,
        })
    }
    fn source(&self) -> PathBuf {
        self.root.join("assets/movement.rhai")
    }
    fn scene(&self, entities: Vec<EntityData>) -> Result<AssetId> {
        let path = self.root.join("assets").join(DEFAULT_SCENE);
        let source = ContentSource {
            base: SceneSnapshot {
                entities,
                ..Default::default()
            },
            ..Default::default()
        };
        fs::write(&path, source.to_json()?)?;
        Ok(SourceMeta::load(&path)?.id)
    }
}
fn entity(asset: AssetId, enabled: bool) -> EntityData {
    let mut entity = EntityData::new("script owner");
    entity.script = Some(Script {
        asset,
        enabled,
        parameters: Default::default(),
    });
    entity
}

#[test]
fn script_source_import_preserves_identity_across_changes_and_rename_without_executing_syntax()
-> Result<()> {
    let fixture = Fixture::new()?;
    let mut store = AssetStore::open(&fixture.products)?;
    assert_eq!(
        store.index().products[&fixture.script].kind,
        AssetKind::Script
    );
    let Product::Script(script) = &store.load(fixture.script)?.payload else {
        panic!("script product")
    };
    assert_eq!(script.source, SOURCE);
    let metadata = fs::read(SourceMeta::path(&fixture.source()))?;
    let digest_before = store.index().products[&fixture.script].digest.clone();
    // Asset production stores text; Runtime is responsible for compiling invalid Rhai syntax.
    let invalid_syntax = "fn definitely_not_valid( {";
    fs::write(fixture.source(), invalid_syntax)?;
    let changed = prepare(&fixture.root, &fixture.products)?;
    assert_eq!(changed.changed, 1);
    store.refresh()?;
    assert_ne!(
        store.index().products[&fixture.script].digest,
        digest_before
    );
    let Product::Script(script) = &store.load(fixture.script)?.payload else {
        panic!("script product")
    };
    assert_eq!(script.source, invalid_syntax);
    assert_eq!(fs::read(SourceMeta::path(&fixture.source()))?, metadata);
    let renamed = fixture.root.join("assets/renamed.rhai");
    fs::rename(fixture.source(), &renamed)?;
    fs::rename(
        SourceMeta::path(&fixture.source()),
        SourceMeta::path(&renamed),
    )?;
    let moved = prepare(&fixture.root, &fixture.products)?;
    assert_eq!(moved.changed, 0);
    assert_eq!(SourceMeta::load(&renamed)?.id, fixture.script);
    let paths = ProjectPaths::open(&fixture.root)?;
    let catalog = SourceCatalog::read(&paths, store.index())?;
    assert_eq!(
        catalog.of_kind(AssetKind::Script).next().unwrap().source,
        PathBuf::from("renamed.rhai")
    );
    Ok(())
}

#[test]
fn invalid_script_bytes_never_replace_the_accepted_index_or_identity() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut store = AssetStore::open(&fixture.products)?;
    let before = fs::read(fixture.products.join("index.json"))?;
    let metadata = fs::read(SourceMeta::path(&fixture.source()))?;
    for bytes in [
        vec![],
        b" \n\t".to_vec(),
        vec![0xff, 0xfe],
        b"let value = \0;".to_vec(),
        vec![b'a'; MAX_SCRIPT_BYTES + 1],
    ] {
        fs::write(fixture.source(), bytes)?;
        assert!(prepare(&fixture.root, &fixture.products).is_err());
        assert_eq!(fs::read(fixture.products.join("index.json"))?, before);
        assert_eq!(fs::read(SourceMeta::path(&fixture.source()))?, metadata);
        let Product::Script(script) = &store.load(fixture.script)?.payload else {
            panic!("script product")
        };
        assert_eq!(script.source, SOURCE);
    }
    fs::write(fixture.source(), SOURCE)?;
    let invalid = fixture.root.join("assets/invalid.rhai");
    fs::write(&invalid, b"\0")?;
    assert!(prepare(&fixture.root, &fixture.products).is_err());
    assert!(!SourceMeta::path(&invalid).exists());
    fs::remove_file(invalid)?;
    prepare(&fixture.root, &fixture.products)?;
    Ok(())
}

#[test]
fn enabled_sources_are_loaded_once_but_disabled_bindings_still_require_valid_script_products()
-> Result<()> {
    let fixture = Fixture::new()?;
    let disabled_path = fixture.root.join("assets/disabled.rhai");
    fs::write(&disabled_path, "fn update(dt) {}")?;
    prepare(&fixture.root, &fixture.products)?;
    let disabled = SourceMeta::load(&disabled_path)?.id;
    let scene = fixture.scene(vec![
        entity(fixture.script, true),
        entity(fixture.script, true),
        entity(disabled, false),
    ])?;
    prepare(&fixture.root, &fixture.products)?;
    let mut store = AssetStore::open(&fixture.products)?;
    let snapshot = store.expand_scene(scene)?;
    store.evict(scene);
    assert_eq!(store.resident_count(), 0);
    let sources = store.script_sources(&snapshot)?;
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[&fixture.script], SOURCE);
    assert_eq!(store.resident_count(), 1);
    for enabled in [false, true] {
        let wrong = SceneSnapshot {
            entities: vec![entity(scene, enabled)],
            ..Default::default()
        };
        assert!(store.script_sources(&wrong).is_err());
        let before = fs::read(fixture.products.join("index.json"))?;
        fixture.scene(wrong.entities)?;
        assert!(prepare(&fixture.root, &fixture.products).is_err());
        assert_eq!(fs::read(fixture.products.join("index.json"))?, before);
    }
    Ok(())
}

#[test]
fn packaged_script_closure_loads_after_authoring_and_development_products_are_removed() -> Result<()>
{
    let fixture = Fixture::new()?;
    let unused_path = fixture.root.join("assets/unused.rhai");
    let disabled_path = fixture.root.join("assets/disabled.rhai");
    fs::write(&unused_path, "let unused = true;")?;
    fs::write(&disabled_path, "let disabled = true;")?;
    prepare(&fixture.root, &fixture.products)?;
    let unused = SourceMeta::load(&unused_path)?.id;
    let disabled = SourceMeta::load(&disabled_path)?.id;
    let scene = fixture.scene(vec![entity(fixture.script, true), entity(disabled, false)])?;
    prepare(&fixture.root, &fixture.products)?;
    let output = fixture._temp.path().join("export");
    package(
        &fixture.products,
        comet_core::GameId::new(),
        scene,
        &std::env::current_exe()?,
        &output,
    )?;
    fs::remove_dir_all(&fixture.root)?;
    fs::remove_dir_all(&fixture.products)?;
    let mut store = AssetStore::open(output.join("products"))?;
    assert!(store.index().products.contains_key(&fixture.script));
    assert!(store.index().products.contains_key(&disabled));
    assert!(!store.index().products.contains_key(&unused));
    assert_eq!(store.dependency_closure(scene)?.len(), 3);
    let snapshot = store.expand_scene(scene)?;
    let scripts = store.script_sources(&snapshot)?;
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[&fixture.script], SOURCE);
    Ok(())
}

#[test]
fn prepare_rejects_unsupported_and_corrupt_indexes_without_replacing_them() -> Result<()> {
    let fixture = Fixture::new()?;
    prepare(&fixture.root, &fixture.products)?;
    let path = fixture.products.join("index.json");
    let rebuilt = AssetStore::open(&fixture.products)?;
    let current = fs::read(&path)?;
    for bytes in [
        b"not json".to_vec(),
        serde_json::to_vec(&serde_json::json!({"version":99,"products":{}}))?,
        {
            let mut bad = rebuilt.index().clone();
            bad.products.values_mut().next().unwrap().digest = "invalid".into();
            serde_json::to_vec(&bad)?
        },
        {
            let mut bad = rebuilt.index().clone();
            bad.products
                .values_mut()
                .next()
                .unwrap()
                .dependencies
                .insert(AssetId::new());
            serde_json::to_vec(&bad)?
        },
    ] {
        fs::write(&path, &bytes)?;
        assert!(prepare(&fixture.root, &fixture.products).is_err());
        assert_eq!(fs::read(&path)?, bytes);
    }
    for version in 0..INDEX_VERSION {
        let mut unsupported = rebuilt.index().clone();
        unsupported.version = version;
        let bytes = serde_json::to_vec(&unsupported)?;
        fs::write(&path, &bytes)?;
        assert!(prepare(&fixture.root, &fixture.products).is_err());
        assert_eq!(fs::read(&path)?, bytes);
    }
    fs::write(&path, current)?;
    prepare(&fixture.root, &fixture.products)?;
    assert_eq!(digest(&fs::read(path)?), rebuilt.revision());
    Ok(())
}
