use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_asset_pipeline::{package, prepare};
use comet_core::AssetId;
use std::{
    fs,
    path::{Path, PathBuf},
};
struct Project {
    _temp: tempfile::TempDir,
    path: PathBuf,
    output: PathBuf,
}
impl Project {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("project");
        let output = temp.path().join("products");
        fs::create_dir_all(path.join("assets"))?;
        for file in fs::read_dir(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/gltf/assets"),
        )? {
            let file = file?;
            if file.file_name() != ".assetc.lock" {
                fs::copy(file.path(), path.join("assets").join(file.file_name()))?;
            }
        }
        Ok(Self {
            _temp: temp,
            path,
            output,
        })
    }
    fn source(&self, name: &str) -> PathBuf {
        self.path.join("assets").join(name)
    }
    fn prepare(&self) -> Result<comet_asset_pipeline::BuildReport> {
        prepare(&self.path, &self.output)
    }
}
fn scene() -> AssetId {
    "40000000-0000-4000-8000-000000000001".parse().unwrap()
}
#[test]
fn static_multimaterial_model_and_external_buffer_updates() -> Result<()> {
    let p = Project::new()?;
    let first = p.prepare()?;
    assert_eq!(first.sources, 2);
    assert_eq!(first.products, 13);
    let mut store = AssetStore::open(&p.output)?;
    let before = store.expand_scene(scene())?;
    assert_eq!(before.entities.len(), 9);
    assert_eq!(
        before.entities.iter().filter(|e| e.model.is_some()).count(),
        5
    );
    let materials: std::collections::BTreeSet<_> = before
        .entities
        .iter()
        .filter_map(|e| e.model.as_ref().map(|m| m.surface.material))
        .collect();
    assert_eq!(materials.len(), 4);
    let mesh = before
        .entities
        .iter()
        .find_map(|e| e.model.as_ref().map(|m| m.mesh))
        .unwrap();
    let old = store.load(mesh)?;
    let mut bytes = fs::read(p.source("showcase.bin"))?;
    bytes[..4].copy_from_slice(&(-0.6_f32).to_le_bytes());
    fs::write(p.source("showcase.bin"), bytes)?;
    assert_eq!(p.prepare()?.changed, 12);
    assert!(store.refresh()?.contains(&mesh));
    assert_ne!(store.load(mesh)?.source_digest, old.source_digest);
    assert_eq!(
        before.entities.iter().map(|e| e.id).collect::<Vec<_>>(),
        store
            .expand_scene(scene())?
            .entities
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(p.prepare()?.changed, 0);
    Ok(())
}
#[test]
fn named_subassets_survive_mesh_and_material_reordering() -> Result<()> {
    let p = Project::new()?;
    p.prepare()?;
    let mut store = AssetStore::open(&p.output)?;
    let before = store.expand_scene(scene())?;
    let mut gltf: serde_json::Value =
        serde_json::from_slice(&fs::read(p.source("showcase.gltf"))?)?;
    gltf["meshes"].as_array_mut().unwrap().swap(0, 1);
    for node in gltf["nodes"].as_array_mut().unwrap() {
        if let Some(index) = node["mesh"].as_u64()
            && index < 2
        {
            node["mesh"] = (1 - index).into();
        }
    }
    let materials = gltf["materials"].as_array_mut().unwrap();
    let count = materials.len() as u64;
    materials.reverse();
    for mesh in gltf["meshes"].as_array_mut().unwrap() {
        for primitive in mesh["primitives"].as_array_mut().unwrap() {
            primitive["material"] = (count - 1 - primitive["material"].as_u64().unwrap()).into();
        }
    }
    fs::write(p.source("showcase.gltf"), serde_json::to_vec(&gltf)?)?;
    p.prepare()?;
    store.refresh()?;
    assert_eq!(store.expand_scene(scene())?, before);
    Ok(())
}
#[test]
fn glb_binary_chunk_and_embedded_image_use_the_same_cpu_protocol() -> Result<()> {
    let p = Project::new()?;
    p.prepare()?;
    let expected = AssetStore::open(&p.output)?.expand_scene(scene())?;
    let mut gltf: serde_json::Value =
        serde_json::from_slice(&fs::read(p.source("showcase.gltf"))?)?;
    gltf["buffers"][0].as_object_mut().unwrap().remove("uri");
    let mut json = serde_json::to_vec(&gltf)?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut binary = fs::read(p.source("showcase.bin"))?;
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let mut glb = b"glTF".to_vec();
    glb.extend(2_u32.to_le_bytes());
    glb.extend(((12 + 8 + json.len() + 8 + binary.len()) as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(0x4e4f534a_u32.to_le_bytes());
    glb.extend(json);
    glb.extend((binary.len() as u32).to_le_bytes());
    glb.extend(0x004e4942_u32.to_le_bytes());
    glb.extend(binary);
    fs::write(p.source("showcase.glb"), glb)?;
    fs::rename(
        p.source("showcase.gltf.meta"),
        p.source("showcase.glb.meta"),
    )?;
    fs::remove_file(p.source("showcase.gltf"))?;
    fs::remove_file(p.source("showcase.bin"))?;
    p.prepare()?;
    assert_eq!(
        AssetStore::open(&p.output)?.expand_scene(scene())?,
        expected
    );
    Ok(())
}
#[test]
fn invalid_external_resources_and_unsupported_features_keep_the_old_index() -> Result<()> {
    let p = Project::new()?;
    p.prepare()?;
    let index = fs::read(p.output.join("index.json"))?;
    let original = fs::read(p.source("showcase.gltf"))?;
    let mut source: serde_json::Value = serde_json::from_slice(&original)?;
    source["buffers"][0]["uri"] = "../outside.bin".into();
    fs::write(
        p.path.join("outside.bin"),
        fs::read(p.source("showcase.bin"))?,
    )?;
    fs::write(p.source("showcase.gltf"), serde_json::to_vec(&source)?)?;
    assert!(format!("{:#}", p.prepare().unwrap_err()).contains("escapes"));
    source = serde_json::from_slice(&original)?;
    source["materials"][0]["alphaMode"] = "BLEND".into();
    fs::write(p.source("showcase.gltf"), serde_json::to_vec(&source)?)?;
    assert!(format!("{:#}", p.prepare().unwrap_err()).contains("opaque"));
    fs::write(p.source("showcase.gltf"), original)?;
    fs::write(p.source("showcase.bin"), b"truncated")?;
    assert!(format!("{:#}", p.prepare().unwrap_err()).contains("truncated"));
    assert_eq!(fs::read(p.output.join("index.json"))?, index);
    Ok(())
}
#[test]
fn package_contains_only_the_entry_closure_and_preserves_existing_destinations() -> Result<()> {
    let p = Project::new()?;
    p.prepare()?;
    let binary = p.path.join("dummy-player");
    fs::write(&binary, b"test executable placeholder")?;
    let output = p.path.join("package");
    package(&p.output, scene(), &binary, &output)?;
    let store = AssetStore::open(output.join("products"))?;
    assert_eq!(store.index().products.len(), 12);
    assert!(output.join("package.json").is_file());
    assert!(!output.join("assets").exists());
    assert!(package(&p.output, scene(), &binary, &output).is_err());
    assert_eq!(
        fs::read(output.join("comet-player"))?,
        b"test executable placeholder"
    );
    let material_count = store
        .index()
        .products
        .values()
        .filter(|p| p.kind == comet_asset_data::product::AssetKind::Material)
        .count();
    assert_eq!(material_count, 4);
    let mut store = AssetStore::open(output.join("products"))?;
    assert!(matches!(store.load(scene())?.payload, Product::Scene(_)));
    Ok(())
}
