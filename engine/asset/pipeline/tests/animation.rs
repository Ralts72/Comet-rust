use anyhow::{Context, Result};
use comet_asset::AssetStore;
use comet_asset_data::{
    AnimationInterpolation, AnimationValues,
    product::{AssetKind, INDEX_VERSION, PRODUCT_VERSION, Product},
};
use comet_asset_pipeline::{SourceMeta, create_project, package, prepare};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{ContentSource, EntityRef, PrefabInstance};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    products: PathBuf,
    source: PathBuf,
    scene: AssetId,
}
impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        let products = temp.path().join("products");
        create_project(&project)?;
        let destination = project.join("assets/model");
        fs::create_dir(&destination)?;
        let shared =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../demo/assets/models/simple_skin");
        for file in fs::read_dir(shared)? {
            let file = file?;
            fs::copy(file.path(), destination.join(file.file_name()))?;
        }
        let source = destination.join("SimpleSkin.gltf");
        let scene = SourceMeta::load(&project.join("assets/main.scene"))?.id;
        let content = ContentSource {
            instances: vec![PrefabInstance {
                id: InstanceId::new(),
                prefab: model(),
                parent: None,
                overrides: vec![],
            }],
            ..Default::default()
        };
        fs::write(project.join("assets/main.scene"), content.to_json()?)?;
        prepare(&project, &products)?;
        Ok(Self {
            _temp: temp,
            project,
            products,
            source,
            scene,
        })
    }
    fn prepare(&self) -> Result<comet_asset_pipeline::BuildReport> {
        prepare(&self.project, &self.products)
    }
}
fn model() -> AssetId {
    "a0000000-0000-4000-8000-000000000010".parse().unwrap()
}

#[test]
fn controller_products_validate_graph_types_and_dependencies_before_atomic_publication()
-> Result<()> {
    let f = Fixture::new()?;
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(f.products.join("index.json"))?)?;
    legacy["version"] = 6.into();
    fs::write(f.products.join("index.json"), serde_json::to_vec(&legacy)?)?;
    assert!(
        AssetStore::open(&f.products).is_err(),
        "runtime requires the current protocol"
    );
    f.prepare()?;
    let source = f.project.join("assets/locomotion.ani");
    let id = AssetId::new();
    fs::write(
        SourceMeta::path(&source),
        serde_json::to_vec(&serde_json::json!({"version":1,"id":id}))?,
    )?;
    let definition = serde_json::json!({
        "version":1,"skeleton":model().child("skeleton:scene"),"initial":"idle",
        "parameters":{"speed":0.,"grounded":true},
        "states":[{"name":"idle","parameter":"speed","looping":true,
            "motions":[{"threshold":0.,"clip":model().child("animation:index:0")},{"threshold":4.,"clip":model().child("animation:index:0")}]}],
        "transitions":[]
    });
    fs::write(&source, serde_json::to_vec(&definition)?)?;
    let mut scene = ContentSource::from_json(&fs::read(f.project.join("assets/main.scene"))?)?;
    scene.base.version = 11;
    let owner = EntityId::in_asset(model(), "animation:controller");
    for (field, value) in [
        (comet_scene::schema::ANIMATOR_CLIP, None),
        (comet_scene::schema::ANIMATOR_CONTROLLER, Some(id)),
    ] {
        scene.instances[0]
            .overrides
            .push(comet_scene::FieldOverride {
                entity: owner,
                field,
                value: comet_scene::schema::FieldValue::OptionalAsset(value),
            });
    }
    fs::write(f.project.join("assets/main.scene"), scene.to_json()?)?;
    f.prepare()?;
    let accepted = fs::read(f.products.join("index.json"))?;
    let metadata = fs::read(SourceMeta::path(&source))?;
    let duplicate =
        serde_json::to_string(&definition)?.replace("\"speed\":0.0", "\"speed\":0.0,\"speed\":1.0");
    fs::write(&source, duplicate)?;
    assert!(f.prepare().is_err());
    assert_eq!(fs::read(f.products.join("index.json"))?, accepted);
    fs::write(&source, serde_json::to_vec(&definition)?)?;
    let mut store = AssetStore::open(&f.products)?;
    let snapshot = store.expand_scene(f.scene)?;
    let libraries = store.animation_sources(&snapshot)?;
    assert_eq!(libraries.controllers.len(), 1);
    assert_eq!(
        libraries.clips.len(),
        1,
        "shared motions retain one immutable clip"
    );
    assert!(store.dependency_closure(f.scene)?.contains(&id));
    let mut cases = Vec::new();
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("initial", serde_json::json!("missing")),
        ("skeleton", serde_json::json!(f.scene)),
    ] {
        let mut bad = definition.clone();
        bad[key] = value;
        cases.push(bad);
    }
    for value in [serde_json::json!(0.), serde_json::json!(-1.)] {
        let mut bad = definition.clone();
        bad["states"][0]["motions"][1]["threshold"] = value;
        cases.push(bad);
    }
    let mut bad = definition.clone();
    bad["states"][0]["parameter"] = serde_json::json!("grounded");
    cases.push(bad);
    let mut bad = definition.clone();
    bad["states"][0]["motions"][0]["clip"] = serde_json::json!(f.scene);
    cases.push(bad);
    let mut bad = definition.clone();
    bad["states"][0]["events"] = serde_json::json!([{"phase":1.1,"name":"bad"}]);
    cases.push(bad);
    let mut bad = definition.clone();
    let duplicate = bad["states"][0].clone();
    bad["states"].as_array_mut().unwrap().push(duplicate);
    cases.push(bad);
    let mut bad = definition.clone();
    bad["transitions"] =
        serde_json::json!([{"from":"idle","to":"missing","duration":0.1,"conditions":[]}]);
    cases.push(bad);
    for bad in cases {
        fs::write(&source, serde_json::to_vec(&bad)?)?;
        assert!(f.prepare().is_err(), "{bad}");
        assert_eq!(fs::read(f.products.join("index.json"))?, accepted);
        assert_eq!(fs::read(SourceMeta::path(&source))?, metadata);
    }
    fs::write(&source, serde_json::to_vec(&definition)?)?;
    assert_eq!(f.prepare()?.changed, 0);
    let mut legacy: serde_json::Value = serde_json::from_slice(&accepted)?;
    legacy["version"] = 6.into();
    let bytes = serde_json::to_vec(&legacy)?;
    fs::write(f.products.join("index.json"), &bytes)?;
    assert!(
        f.prepare().is_err(),
        "index 6 cannot contain controller products"
    );
    assert_eq!(fs::read(f.products.join("index.json"))?, bytes);
    Ok(())
}

#[test]
fn animation_event_import_rejects_bad_tracks_before_replacing_the_accepted_product_index()
-> Result<()> {
    let f = Fixture::new()?;
    let original = fs::read(&f.source)?;
    let metadata = fs::read(SourceMeta::path(&f.source))?;
    let index = fs::read(f.products.join("index.json"))?;
    let mut store = AssetStore::open(&f.products)?;
    let old = store.load(model().child("animation:index:0"))?;
    let Product::AnimationClip(clip) = &old.payload else {
        panic!("animation clip")
    };
    assert_eq!(clip.version, 3);
    assert_eq!(clip.events.len(), 1);
    for events in [
        serde_json::json!([{"phase":-0.1,"name":"bad"}]),
        serde_json::json!([{"phase":1.1,"name":"bad"}]),
        serde_json::json!([{"phase":0.8,"name":"later"},{"phase":0.2,"name":"earlier"}]),
        serde_json::json!([{"phase":0.5,"name":"hit","unknown":true}]),
        serde_json::json!(null),
    ] {
        let mut source: serde_json::Value = serde_json::from_slice(&original)?;
        source["animations"][0]["extras"]["comet_events"] = events;
        fs::write(&f.source, serde_json::to_vec(&source)?)?;
        assert!(f.prepare().is_err());
        assert_eq!(fs::read(f.products.join("index.json"))?, index);
        assert_eq!(fs::read(SourceMeta::path(&f.source))?, metadata);
        assert!(std::sync::Arc::ptr_eq(
            &old,
            &store.load(model().child("animation:index:0"))?
        ));
    }
    let mut source: serde_json::Value = serde_json::from_slice(&original)?;
    source["animations"][0]["extras"]["comet_events"] =
        serde_json::json!([{"phase":0.25,"name":"脚步"},{"phase":1.0,"name":"动作完成"}]);
    fs::write(&f.source, serde_json::to_vec(&source)?)?;
    f.prepare()?;
    store.refresh()?;
    let new = store.load(model().child("animation:index:0"))?;
    let Product::AnimationClip(new) = &new.payload else {
        panic!("animation clip")
    };
    assert_eq!(new.events.len(), 2);
    assert_eq!(new.events[0].name, "脚步");
    assert_eq!(
        clip.events.len(),
        1,
        "old product stays immutable across acceptance"
    );
    assert_eq!(fs::read(SourceMeta::path(&f.source))?, metadata);
    Ok(())
}

#[test]
fn official_skin_builds_typed_products_and_scoped_bindings_then_loads_without_author_sources()
-> Result<()> {
    let f = Fixture::new()?;
    let metadata = fs::read(SourceMeta::path(&f.source))?;
    let mut store = AssetStore::open(&f.products)?;
    assert_eq!(store.index().version, INDEX_VERSION);
    assert_eq!(store.index().products.len(), 7);
    let skeleton = store.load(model().child("skeleton:scene"))?;
    assert_eq!(skeleton.version, PRODUCT_VERSION);
    let Product::Skeleton(skeleton) = &skeleton.payload else {
        panic!("skeleton")
    };
    assert_eq!(skeleton.nodes.len(), 3);
    assert!(skeleton.nodes.iter().all(|node| !node.name.is_empty()));
    let skin = store.load(model().child("skin:index:0"))?;
    let Product::Skin(skin) = &skin.payload else {
        panic!("skin")
    };
    assert_eq!(
        skin.joints
            .iter()
            .map(|joint| joint.node)
            .collect::<Vec<_>>(),
        [
            EntityId::in_asset(model(), "node:index:1"),
            EntityId::in_asset(model(), "node:index:2"),
        ]
    );
    skin.validate_for(skeleton)?;
    let mesh = store.load(model().child("mesh:index:0/primitive:0"))?;
    let Product::Mesh(mesh) = &mesh.payload else {
        panic!("mesh")
    };
    assert_eq!(
        (mesh.version, mesh.positions.len(), mesh.indices.len()),
        (2, 10, 24)
    );
    skin.validate_mesh(mesh)?;
    let clip = store.load(model().child("animation:index:0"))?;
    let Product::AnimationClip(clip) = &clip.payload else {
        panic!("clip")
    };
    assert_eq!(clip.duration, 5.5);
    assert_eq!(clip.channels.len(), 1);
    assert_eq!(clip.channels[0].times.len(), 12);
    assert!(matches!(
        clip.channels[0].values,
        AnimationValues::Rotation(_)
    ));
    let snapshot = store.expand_scene(f.scene)?;
    assert_eq!(snapshot.version, comet_scene::scene::SCENE_VERSION);
    let owner = snapshot
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .context("controller")?;
    let animator = owner.animator.as_ref().unwrap();
    assert_eq!(animator.bindings.len(), 3);
    for name in ["Node 0", "Node 1", "Node 2"] {
        assert!(snapshot.entities.iter().any(|entity| entity.name == name));
    }
    for (source, target) in &animator.bindings {
        let Some(EntityRef::Local { entity }) = target else {
            panic!("local binding")
        };
        assert_ne!(source, entity);
        assert!(snapshot.entities.iter().any(|node| node.id == *entity));
    }
    let libraries = store.animation_sources(&snapshot)?;
    let (skeletons, clips) = (libraries.skeletons, libraries.clips);
    assert_eq!((skeletons.len(), clips.len()), (1, 1));
    assert_eq!(f.prepare()?.changed, 0);
    assert_eq!(fs::read(SourceMeta::path(&f.source))?, metadata);
    let package_path = f._temp.path().join("game");
    package(
        &f.products,
        comet_core::GameId::new(),
        f.scene,
        &std::env::current_exe()?,
        &package_path,
    )?;
    let moved = f._temp.path().join("relocated/game");
    fs::create_dir_all(moved.parent().unwrap())?;
    fs::rename(&package_path, &moved)?;
    fs::remove_dir_all(&f.project)?;
    fs::remove_dir_all(&f.products)?;
    let mut packaged = AssetStore::open(moved.join("products"))?;
    let snapshot = packaged.expand_scene(f.scene)?;
    assert_eq!(packaged.animation_sources(&snapshot)?.clips.len(), 1);
    assert!(
        packaged
            .index()
            .products
            .values()
            .all(|entry| entry.editor.is_none())
    );
    assert!(!moved.join("assets").exists());
    Ok(())
}

#[test]
fn cubic_rotation_import_preserves_derivatives_and_rejects_malformed_updates_atomically()
-> Result<()> {
    let f = Fixture::new()?;
    let metadata = fs::read(SourceMeta::path(&f.source))?;
    let mut source: serde_json::Value = serde_json::from_slice(&fs::read(&f.source)?)?;
    let values = vec![
        [0f32; 4],
        [0., 0., 0., 1.],
        [0., 0., 4., 0.],
        [0.; 4],
        [0., 0., 0., -1.],
        [0.; 4],
    ];
    let mut bytes: Vec<_> = [0f32, 2.].into_iter().flat_map(f32::to_le_bytes).collect();
    bytes.extend(values.iter().flatten().flat_map(|v| v.to_le_bytes()));
    let buffer = f.source.parent().unwrap().join("SimpleSkin_animation.bin");
    fs::write(&buffer, &bytes)?;
    source["buffers"][3]["byteLength"] = bytes.len().into();
    source["bufferViews"][4]["byteLength"] = bytes.len().into();
    source["accessors"][5]["count"] = 2.into();
    source["accessors"][5]["max"] = serde_json::json!([2.]);
    source["accessors"][6]["byteOffset"] = 8.into();
    source["accessors"][6]["count"] = 6.into();
    source["accessors"][6]
        .as_object_mut()
        .unwrap()
        .remove("min");
    source["accessors"][6]
        .as_object_mut()
        .unwrap()
        .remove("max");
    source["animations"][0]["samplers"][0]["interpolation"] = "CUBICSPLINE".into();
    // Keep this regression on the event-free v2 format, independently of the demo's v3 markers.
    source["animations"][0]
        .as_object_mut()
        .unwrap()
        .remove("extras");
    fs::write(&f.source, serde_json::to_vec(&source)?)?;
    f.prepare()?;
    let mut store = AssetStore::open(&f.products)?;
    let clip = store.load(model().child("animation:index:0"))?;
    let Product::AnimationClip(clip) = &clip.payload else {
        panic!("clip")
    };
    assert_eq!(clip.version, 2);
    assert_eq!(
        clip.channels[0].interpolation,
        AnimationInterpolation::CubicSpline
    );
    assert_eq!(clip.channels[0].values, AnimationValues::Rotation(values));
    assert_eq!(f.prepare()?.changed, 0);
    assert_eq!(fs::read(SourceMeta::path(&f.source))?, metadata);
    let index = fs::read(f.products.join("index.json"))?;
    let mut malformed = source.clone();
    malformed["accessors"][6]["count"] = 5.into();
    fs::write(&f.source, serde_json::to_vec(&malformed)?)?;
    assert!(f.prepare().is_err());
    assert_eq!(fs::read(f.products.join("index.json"))?, index);
    fs::write(&f.source, serde_json::to_vec(&source)?)?;
    bytes[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
    fs::write(&buffer, bytes)?;
    assert!(f.prepare().is_err());
    assert_eq!(fs::read(f.products.join("index.json"))?, index);
    assert_eq!(fs::read(SourceMeta::path(&f.source))?, metadata);
    Ok(())
}

#[test]
fn unsupported_and_malformed_animation_inputs_preserve_the_index_and_source_identity() -> Result<()>
{
    let f = Fixture::new()?;
    let index = fs::read(f.products.join("index.json"))?;
    let metadata = fs::read(SourceMeta::path(&f.source))?;
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&f.source)?)?;
    let mut variants = Vec::new();
    let mut changed = original.clone();
    changed["animations"][0]["samplers"][0]["interpolation"] = "CUBICSPLINE".into();
    variants.push(changed);
    let mut changed = original.clone();
    changed["skins"][0]["joints"] = serde_json::json!([1, 1]);
    variants.push(changed);
    let mut changed = original.clone();
    changed["meshes"][0]["primitives"][0]["attributes"]["JOINTS_1"] = 2.into();
    changed["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_1"] = 3.into();
    variants.push(changed);
    let mut changed = original.clone();
    changed["meshes"][0]["primitives"][0]["targets"] = serde_json::json!([{"POSITION":1}]);
    variants.push(changed);
    let mut changed = original.clone();
    changed["scenes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"nodes":[]}));
    variants.push(changed);
    let mut changed = original.clone();
    changed["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({}));
    changed["skins"][0]["joints"] = serde_json::json!([1, 3]);
    variants.push(changed);
    let mut changed = original.clone();
    changed["buffers"][0]["byteLength"] = (comet_asset_data::MAX_ANIMATION_BYTES + 1).into();
    variants.push(changed);
    for value in variants {
        fs::write(&f.source, serde_json::to_vec(&value)?)?;
        assert!(f.prepare().is_err());
        assert_eq!(fs::read(f.products.join("index.json"))?, index);
        assert_eq!(fs::read(SourceMeta::path(&f.source))?, metadata);
    }
    let mut unequal_attributes = original.clone();
    unequal_attributes["accessors"][2]["count"] = 9.into();
    unequal_attributes["accessors"][3]["count"] = 9.into();
    fs::write(&f.source, serde_json::to_vec(&unequal_attributes)?)?;
    let error = f.prepare().unwrap_err();
    assert!(format!("{error:#}").contains("vertex attribute count mismatch before decoding"));
    assert_eq!(fs::read(f.products.join("index.json"))?, index);
    fs::write(&f.source, serde_json::to_vec(&original)?)?;
    for (name, offset, bytes) in [
        (
            "SimpleSkin_skinningData.bin",
            0,
            128u16.to_le_bytes().to_vec(),
        ),
        (
            "SimpleSkin_skinningData.bin",
            160,
            (-1f32).to_le_bytes().to_vec(),
        ),
        (
            "SimpleSkin_inverseBindMatrices.bin",
            12,
            1f32.to_le_bytes().to_vec(),
        ),
    ] {
        let path = f.source.parent().unwrap().join(name);
        let original = fs::read(&path)?;
        let mut changed = original.clone();
        changed[offset..offset + bytes.len()].copy_from_slice(&bytes);
        fs::write(&path, changed)?;
        assert!(f.prepare().is_err(), "{name}:{offset}");
        assert_eq!(fs::read(f.products.join("index.json"))?, index);
        fs::write(path, original)?;
    }
    f.prepare()?;
    Ok(())
}

#[test]
fn author_unbound_rigs_remain_saveable_but_dangling_bindings_and_legacy_animation_caches_are_rejected()
-> Result<()> {
    let f = Fixture::new()?;
    let mut store = AssetStore::open(&f.products)?;
    let product = store.load(model())?;
    let Product::Prefab(mut prefab) = product.payload.clone() else {
        panic!("prefab")
    };
    let animator = prefab
        .base
        .entities
        .iter_mut()
        .find_map(|entity| entity.animator.as_mut())
        .unwrap();
    *animator.bindings.values_mut().next().unwrap() = None;
    let path = f.project.join("assets/unbound.prefab");
    fs::write(&path, prefab.to_json()?)?;
    f.prepare()?;
    let mut store = AssetStore::open(&f.products)?;
    let source = SourceMeta::load(&path)?.id;
    let snapshot = match &store.load(source)?.payload {
        Product::Prefab(value) => value.expand(&mut store)?,
        _ => unreachable!(),
    };
    assert_eq!(store.animation_sources(&snapshot)?.skeletons.len(), 1);
    let index_path = f.products.join("index.json");
    let accepted = fs::read(&index_path)?;
    let animator = prefab
        .base
        .entities
        .iter_mut()
        .find_map(|entity| entity.animator.as_mut())
        .unwrap();
    *animator.bindings.values_mut().next().unwrap() = Some(EntityRef::local(EntityId::new()));
    assert!(prefab.to_json().is_err());
    fs::write(&path, serde_json::to_vec(&prefab)?)?;
    assert!(f.prepare().is_err());
    assert_eq!(fs::read(&index_path)?, accepted);
    fs::remove_file(&path)?;
    fs::remove_file(SourceMeta::path(&path))?;
    f.prepare()?;
    let current: serde_json::Value = serde_json::from_slice(&fs::read(&index_path)?)?;
    for version in 1..=5 {
        let mut legacy = current.clone();
        legacy["version"] = version.into();
        if version < 3 {
            for entry in legacy["products"].as_object_mut().unwrap().values_mut() {
                entry.as_object_mut().unwrap().remove("editor");
            }
        }
        let bytes = serde_json::to_vec(&legacy)?;
        fs::write(&index_path, &bytes)?;
        assert!(f.prepare().is_err());
        assert_eq!(fs::read(&index_path)?, bytes);
    }
    let mut legacy_mesh = current.clone();
    legacy_mesh["version"] = 5.into();
    legacy_mesh["products"]
        .as_object_mut()
        .unwrap()
        .retain(|_, entry| entry["kind"] == "mesh");
    for entry in legacy_mesh["products"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        entry.as_object_mut().unwrap().remove("editor");
    }
    let bytes = serde_json::to_vec(&legacy_mesh)?;
    fs::write(&index_path, &bytes)?;
    let error = f.prepare().unwrap_err();
    assert!(format!("{error:#}").contains("newer mesh skin attributes"));
    assert_eq!(fs::read(&index_path)?, bytes);
    fs::write(&index_path, serde_json::to_vec(&current)?)?;
    assert!(
        AssetStore::open(&f.products)?
            .index()
            .products
            .values()
            .any(|entry| entry.kind == AssetKind::AnimationClip)
    );
    Ok(())
}
