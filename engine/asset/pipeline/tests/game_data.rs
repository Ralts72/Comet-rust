use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::{game_data::DataValue, product::AssetKind};
use comet_asset_pipeline::{
    DEFAULT_SCENE, SourceCatalog, SourceMeta, create_project, package, prepare,
};
use comet_core::{AssetId, GameId, InstanceId, ProjectPaths};
use comet_scene::{
    ContentSource, EntityData, FieldOverride, PrefabInstance, SceneSnapshot, Script, ScriptValue,
    schema,
};
use serde_json::json;
use std::{fs, path::Path};
fn source(path: &Path, id: AssetId, bytes: impl AsRef<[u8]>) -> Result<()> {
    fs::write(path, bytes)?;
    fs::write(
        SourceMeta::path(path),
        serde_json::to_vec(&SourceMeta { version: 1, id })?,
    )?;
    Ok(())
}
#[test]
fn source_validation_prefab_data_closure_and_failed_publication_keep_identity_and_portable_products()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    let products = temp.path().join("products");
    create_project(&root)?;
    let (leaf, parent, script, prefab) = (
        AssetId::new(),
        AssetId::new(),
        AssetId::new(),
        AssetId::new(),
    );
    let child = root.join("assets/leaf.csv");
    let schema = root.join("assets/leaf.csv.schema.json");
    let table = root.join("assets/stats.data.json");
    source(&child, leaf, b"id,hp\nhero,7\n")?;
    fs::write(&schema, br#"{"fields":{"hp":{"type":"integer","min":1}}}"#)?;
    let value = json!({"fields":{"next":{"type":"asset","asset_type":"data"},"text":{"type":"json"}},"rows":[{"id":"link","next":leaf,"text":{"not_an_asset":AssetId::new()}}]});
    source(&table, parent, serde_json::to_vec(&value)?)?;
    source(
        &root.join("assets/script.rhai"),
        script,
        b"fn on_start(ctx) {}",
    )?;
    let mut actor = EntityData::new("owner");
    actor.script = Some(Script::new(script));
    source(
        &root.join("assets/actor.prefab"),
        prefab,
        ContentSource {
            base: SceneSnapshot {
                version: 4,
                entities: vec![actor.clone()],
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    let content = ContentSource {
        instances: vec![PrefabInstance {
            id: InstanceId::new(),
            prefab,
            parent: None,
            overrides: vec![FieldOverride {
                entity: actor.id,
                field: schema::SCRIPT_PARAMETERS,
                value: schema::FieldValue::ScriptParameters(
                    [("stats".into(), ScriptValue::Data(Some(parent)))].into(),
                ),
            }],
        }],
        ..Default::default()
    };
    let path = root.join("assets").join(DEFAULT_SCENE);
    fs::write(&path, content.to_json()?)?;
    let scene = SourceMeta::load(&path)?.id;
    prepare(&root, &products)?;
    let mut store = AssetStore::open(&products)?;
    let expanded = store.expand_scene(scene)?;
    assert_eq!(expanded.version, 12);
    let mut unsupported = expanded.clone();
    unsupported.version = 11;
    assert!(store.scene_resources(&unsupported).is_err());
    let library = store.scene_resources(&expanded)?.data;
    assert_eq!(library.len(), 2);
    assert_eq!(library[&leaf].rows["hero"]["hp"], DataValue::Integer(7));
    assert_eq!(
        store.dependency_closure(scene)?,
        [scene, prefab, script, parent, leaf].into()
    );
    let metadata = fs::read(SourceMeta::path(&table))?;
    let index = fs::read(products.join("index.json"))?;
    let child_metadata = fs::read(SourceMeta::path(&child))?;
    fs::write(&schema, br#"{"fields":{"hp":{"type":"integer","max":6}}}"#)?;
    assert!(prepare(&root, &products).is_err());
    assert_eq!(fs::read(products.join("index.json"))?, index);
    fs::File::create(&schema)?.set_len(comet_asset_data::game_data::MAX_DATA_BYTES as u64 + 1)?;
    assert!(prepare(&root, &products).is_err());
    assert_eq!(fs::read(products.join("index.json"))?, index);
    fs::write(&schema, br#"{"fields":{"hp":{"type":"integer","min":1}}}"#)?;
    let mut legacy: serde_json::Value = serde_json::from_slice(&index)?;
    legacy["version"] = json!(6);
    let old = serde_json::to_vec(&legacy)?;
    fs::write(products.join("index.json"), &old)?;
    assert!(format!("{:#}", prepare(&root, &products).unwrap_err()).contains("newer game data"));
    assert_eq!(fs::read(products.join("index.json"))?, old);
    fs::write(products.join("index.json"), &index)?;
    let missing = root.join("assets/missing.csv");
    fs::write(&missing, b"id,hp\nhero,1\n")?;
    assert!(format!("{:#}", prepare(&root, &products).unwrap_err()).contains("CSV schema"));
    assert!(!SourceMeta::path(&missing).exists());
    assert_eq!(fs::read(products.join("index.json"))?, index);
    fs::remove_file(missing)?;
    let mut wrong = value.clone();
    wrong["fields"]["next"]["asset_type"] = json!("prefab");
    fs::write(&table, serde_json::to_vec(&wrong)?)?;
    assert!(prepare(&root, &products).is_err());
    assert_eq!(fs::read(products.join("index.json"))?, index);
    assert_eq!(fs::read(SourceMeta::path(&table))?, metadata);
    fs::write(&table, serde_json::to_vec(&value)?)?;
    let invalid = root.join("assets/bad.data.json");
    fs::write(&invalid, b"{\"fields\":{},\"rows\":[],\"rows\":[]}")?;
    assert!(prepare(&root, &products).is_err());
    assert!(!SourceMeta::path(&invalid).exists());
    assert_eq!(fs::read(products.join("index.json"))?, index);
    fs::remove_file(invalid)?;
    let renamed = root.join("assets/moved.data.json");
    fs::rename(&table, &renamed)?;
    fs::rename(SourceMeta::path(&table), SourceMeta::path(&renamed))?;
    let old_digest = store.index().products[&leaf].digest.clone();
    fs::write(&schema, br#"{"fields":{"hp":{"type":"integer","min":2}}}"#)?;
    prepare(&root, &products)?;
    store.refresh()?;
    assert_ne!(
        store.index().products[&leaf].digest,
        old_digest,
        "schema-only edits rebuild data"
    );
    assert_eq!(fs::read(SourceMeta::path(&child))?, child_metadata);
    let old_digest = store.index().products[&leaf].digest.clone();
    fs::write(
        &schema,
        b"{ \"fields\" : {\"hp\":{\"type\":\"integer\",\"min\":2}} }\n",
    )?;
    prepare(&root, &products)?;
    store.refresh()?;
    assert_ne!(
        store.index().products[&leaf].digest,
        old_digest,
        "schema bytes participate in the source fingerprint"
    );
    let moved = root.join("assets/moved.csv");
    fs::rename(&child, &moved)?;
    fs::rename(&schema, root.join("assets/moved.csv.schema.json"))?;
    fs::rename(SourceMeta::path(&child), SourceMeta::path(&moved))?;
    prepare(&root, &products)?;
    store.refresh()?;
    assert_eq!(SourceMeta::load(&moved)?.id, leaf);
    assert_eq!(SourceMeta::load(&renamed)?.id, parent);
    let catalog = SourceCatalog::read(&ProjectPaths::open(&root)?, store.index())?;
    assert_eq!(catalog.of_kind(AssetKind::Data).count(), 2);
    let export = temp.path().join("game");
    package(
        &products,
        GameId::new(),
        scene,
        &std::env::current_exe()?,
        &export,
    )?;
    fs::remove_dir_all(&root)?;
    fs::remove_dir_all(&products)?;
    let mut packaged = AssetStore::open(export.join("products"))?;
    let snapshot = packaged.expand_scene(scene)?;
    assert_eq!(packaged.scene_resources(&snapshot)?.data.len(), 2);
    assert_eq!(library[&leaf].rows["hero"]["hp"], DataValue::Integer(7));
    Ok(())
}
