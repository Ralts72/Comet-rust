use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::product::{MeshData, Product};
use comet_asset_pipeline::{package, prepare};
use comet_core::AssetId;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
struct Project {
    _temp: tempfile::TempDir,
    path: PathBuf,
    output: PathBuf,
}
impl Project {
    fn empty() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("project");
        let output = temp.path().join("products");
        fs::create_dir_all(path.join("assets"))?;
        Ok(Self {
            _temp: temp,
            path,
            output,
        })
    }
    fn new() -> Result<Self> {
        let project = Self::empty()?;
        for file in fs::read_dir(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/gltf/assets"),
        )? {
            let file = file?;
            if file.file_name() != ".assetc.lock" {
                fs::copy(
                    file.path(),
                    project.path.join("assets").join(file.file_name()),
                )?;
            }
        }
        Ok(project)
    }
    fn source(&self, name: &str) -> PathBuf {
        self.path.join("assets").join(name)
    }
    fn prepare(&self) -> Result<comet_asset_pipeline::BuildReport> {
        prepare(&self.path, &self.output)
    }
    fn for_normals() -> Result<Self> {
        let project = Self::empty()?;
        fs::write(
            project.source("triangle.gltf.meta"),
            serde_json::to_vec(&serde_json::json!({"version": 1, "id": normal_asset()}))?,
        )?;
        Ok(project)
    }
    fn write_mesh(&self, positions: &[[f32; 3]], indices: &[u32]) -> Result<()> {
        let mut bytes = positions
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let index_offset = bytes.len();
        bytes.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
        let min = std::array::from_fn::<_, 3, _>(|axis| {
            positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min)
        });
        let max = std::array::from_fn::<_, 3, _>(|axis| {
            positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        fs::write(self.source("triangle.bin"), &bytes)?;
        fs::write(
            self.source("triangle.gltf"),
            serde_json::to_vec(&serde_json::json!({
                "asset": {"version": "2.0"},
                "buffers": [{"uri": "triangle.bin", "byteLength": bytes.len()}],
                "bufferViews": [
                    {"buffer": 0, "byteOffset": 0, "byteLength": index_offset},
                    {"buffer": 0, "byteOffset": index_offset, "byteLength": bytes.len() - index_offset}
                ],
                "accessors": [
                    {"bufferView": 0, "componentType": 5126, "count": positions.len(),
                        "type": "VEC3", "min": min, "max": max},
                    {"bufferView": 1, "componentType": 5125, "count": indices.len(), "type": "SCALAR"}
                ],
                "meshes": [{"name": "Triangle", "primitives": [{
                    "attributes": {"POSITION": 0}, "indices": 1
                }]}],
                "nodes": [{"name": "TriangleNode", "mesh": 0}],
                "scenes": [{"nodes": [0]}], "scene": 0
            }))?,
        )?;
        Ok(())
    }
    fn load_mesh(&self) -> Result<MeshData> {
        let mut store = AssetStore::open(&self.output)?;
        match &store
            .load(normal_asset().child("mesh:name:Triangle/primitive:0"))?
            .payload
        {
            Product::Mesh(mesh) => Ok(mesh.clone()),
            _ => panic!("expected the generated glTF mesh"),
        }
    }
}
fn scene() -> AssetId {
    "40000000-0000-4000-8000-000000000001".parse().unwrap()
}
fn normal_asset() -> AssetId {
    "40000000-0000-4000-8000-000000000003".parse().unwrap()
}
fn assert_normal(actual: [f32; 3], expected: [f32; 3]) {
    assert!(actual.iter().all(|v| v.is_finite()));
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
}
#[test]
fn generated_normals_are_scale_independent_and_keep_subasset_identity() -> Result<()> {
    let p = Project::for_normals()?;
    let metadata = fs::read(p.source("triangle.gltf.meta"))?;
    let mut identity = None;
    for side in [1e-30_f32, 1e-20, 1., 1e10, 1e20] {
        p.write_mesh(&[[0.; 3], [0., side, 0.], [0., 0., side]], &[0, 1, 2])?;
        let report = p.prepare()?;
        assert_eq!((report.sources, report.products), (1, 3));
        for normal in p.load_mesh()?.normals {
            assert_normal(normal, [1., 0., 0.]);
        }
        let mut store = AssetStore::open(&p.output)?;
        let topology = store
            .index()
            .products
            .iter()
            .map(|(id, entry)| (*id, entry.kind, entry.dependencies.clone()))
            .collect::<Vec<_>>();
        let prefab = store.load(normal_asset())?.payload.clone();
        let current = (topology, prefab);
        if let Some(identity) = &identity {
            assert_eq!(&current, identity);
        } else {
            identity = Some(current);
        }
        assert_eq!(fs::read(p.source("triangle.gltf.meta"))?, metadata);
        assert_eq!(p.prepare()?.changed, 0);
    }
    Ok(())
}
#[test]
fn generated_normals_keep_area_weighting_at_extreme_scales() -> Result<()> {
    let p = Project::for_normals()?;
    for side in [1e-30_f32, 1., 1e20] {
        p.write_mesh(
            &[[0.; 3], [0., side, 0.], [0., 0., side], [2. * side, 0., 0.]],
            &[0, 1, 2, 0, 2, 3],
        )?;
        p.prepare()?;
        let normals = p.load_mesh()?.normals;
        let weighted = [1. / 5_f32.sqrt(), 2. / 5_f32.sqrt(), 0.];
        assert_normal(normals[0], weighted);
        assert_normal(normals[1], [1., 0., 0.]);
        assert_normal(normals[2], weighted);
        assert_normal(normals[3], [0., 1., 0.]);
    }
    Ok(())
}
#[test]
fn degenerate_cancelled_and_unreferenced_normals_stay_zero() -> Result<()> {
    let p = Project::for_normals()?;
    for side in [1e-30_f32, 1., 1e20] {
        p.write_mesh(&[[0.; 3], [0., side, 0.], [0., 2. * side, 0.]], &[0, 1, 2])?;
        p.prepare()?;
        assert!(p.load_mesh()?.normals.iter().all(|n| *n == [0.; 3]));
        p.write_mesh(
            &[[0.; 3], [0., side, 0.], [0., 0., side], [side, 0., 0.]],
            &[0, 1, 2, 0, 2, 1],
        )?;
        p.prepare()?;
        assert!(p.load_mesh()?.normals.iter().all(|n| *n == [0.; 3]));
    }
    Ok(())
}
#[test]
fn normal_generation_widens_before_subtracting_finite_coordinates() -> Result<()> {
    let p = Project::for_normals()?;
    p.write_mesh(
        &[[0., -2e38, 0.], [0., 2e38, 0.], [0., -2e38, 2e38]],
        &[0, 1, 2],
    )?;
    p.prepare()?;
    for normal in p.load_mesh()?.normals {
        assert_normal(normal, [1., 0., 0.]);
    }
    Ok(())
}
#[test]
fn source_catalog_rejects_derived_identities_without_restricting_source_renames() -> Result<()> {
    use comet_asset_data::product::AssetKind;
    use comet_asset_pipeline::{SourceCatalog, SourceMeta};

    let p = Project::new()?;
    p.prepare()?;
    let paths = comet_core::ProjectPaths::open(&p.path)?;
    let mut store = AssetStore::open(&p.output)?;
    let model = SourceMeta::load(&p.source("showcase.gltf"))?.id;
    let mesh = model.child("mesh:name:AmberTwoMaterials/primitive:0");
    let product = store.load(mesh)?;
    let Product::Mesh(data) = &product.payload else {
        panic!("expected a derived mesh");
    };
    let alias = p.source("unbuilt.mesh");
    fs::write(&alias, serde_json::to_vec(data)?)?;
    fs::write(
        SourceMeta::path(&alias),
        serde_json::to_vec(&SourceMeta {
            version: 1,
            id: mesh,
        })?,
    )?;
    let error = SourceCatalog::read(&paths, store.index())
        .err()
        .expect("a derived product must not become an author source");
    assert!(format!("{error:#}").contains("derived product"));
    assert_eq!(SourceMeta::load(&alias)?.id, mesh);
    fs::remove_file(SourceMeta::path(&alias))?;
    fs::remove_file(&alias)?;

    let renamed = p.source("renamed.gltf");
    fs::rename(p.source("showcase.gltf"), &renamed)?;
    fs::rename(p.source("showcase.gltf.meta"), SourceMeta::path(&renamed))?;
    let catalog = SourceCatalog::read(&paths, store.index())?;
    assert_eq!(
        catalog
            .of_kind(AssetKind::Prefab)
            .find(|source| source.id == model)
            .unwrap()
            .source,
        Path::new("renamed.gltf")
    );
    let mut unlabelled = store.index().clone();
    for entry in unlabelled.products.values_mut() {
        entry.editor = None;
    }
    unlabelled.validate()?;
    assert!(
        SourceCatalog::read(&paths, &unlabelled)?
            .of_kind(AssetKind::Prefab)
            .any(|source| source.id == model)
    );
    Ok(())
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
    let provenance: BTreeMap<_, _> = store
        .index()
        .products
        .iter()
        .map(|(id, entry)| (*id, entry.editor.clone().unwrap()))
        .collect();
    let model = comet_asset_pipeline::SourceMeta::load(&p.source("showcase.gltf"))?.id;
    assert_eq!(provenance[&model].source, model);
    assert_eq!(provenance[&model].label, "showcase.gltf");
    for (key, label) in [
        (
            "mesh:name:AmberTwoMaterials/primitive:0",
            "AmberTwoMaterials / primitive 0",
        ),
        (
            "mesh:name:AmberTwoMaterials/primitive:1",
            "AmberTwoMaterials / primitive 1",
        ),
        ("material:name:Amber", "Amber"),
        ("image:name:Checker", "Checker"),
        ("material:default", "Default material"),
    ] {
        let info = &provenance[&model.child(key)];
        assert_eq!(info.source, model);
        assert_eq!(info.label, label);
    }
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
    assert_eq!(
        store
            .index()
            .products
            .iter()
            .map(|(id, entry)| (*id, entry.editor.clone().unwrap()))
            .collect::<BTreeMap<_, _>>(),
        provenance
    );
    Ok(())
}

#[test]
fn unnamed_and_duplicate_subassets_have_distinct_labels_without_changing_key_rules() -> Result<()> {
    let p = Project::new()?;
    let mut gltf: serde_json::Value =
        serde_json::from_slice(&fs::read(p.source("showcase.gltf"))?)?;
    for mesh in &mut gltf["meshes"].as_array_mut().unwrap()[..2] {
        mesh["name"] = "Shared mesh".into();
    }
    gltf["meshes"][2].as_object_mut().unwrap().remove("name");
    for material in &mut gltf["materials"].as_array_mut().unwrap()[..2] {
        material["name"] = "Shared material".into();
    }
    gltf["materials"][2].as_object_mut().unwrap().remove("name");
    gltf["images"][0].as_object_mut().unwrap().remove("name");
    fs::write(p.source("showcase.gltf"), serde_json::to_vec(&gltf)?)?;
    p.prepare()?;
    let store = AssetStore::open(&p.output)?;
    let model = comet_asset_pipeline::SourceMeta::load(&p.source("showcase.gltf"))?.id;
    for (key, label) in [
        (
            "mesh:index:0/primitive:0",
            "Shared mesh (Mesh 0) / primitive 0",
        ),
        (
            "mesh:index:1/primitive:0",
            "Shared mesh (Mesh 1) / primitive 0",
        ),
        ("mesh:index:2/primitive:0", "Mesh 2 / primitive 0"),
        ("material:index:0", "Shared material (Material 0)"),
        ("material:index:1", "Shared material (Material 1)"),
        ("material:index:2", "Material 2"),
        ("image:index:0", "Image 0"),
    ] {
        let editor = store.index().products[&model.child(key)]
            .editor
            .as_ref()
            .unwrap();
        assert_eq!(editor.source, model);
        assert_eq!(editor.label, label);
    }
    Ok(())
}

#[test]
fn direct_subproduct_packages_strip_editor_provenance_without_collecting_the_source_root()
-> Result<()> {
    use comet_scene::{ContentSource, EntityData, Model, Surface};
    let p = Project::new()?;
    let model = comet_asset_pipeline::SourceMeta::load(&p.source("showcase.gltf"))?.id;
    let mesh = model.child("mesh:name:AmberTwoMaterials/primitive:0");
    let material = model.child("material:name:Amber");
    let mut entity = EntityData::new("Direct mesh");
    entity.model = Some(Model {
        mesh,
        surface: Surface {
            material,
            roughness: 0.5,
        },
    });
    let mut source = ContentSource::default();
    source.base.entities.push(entity);
    fs::write(p.source("main.scene"), source.to_json()?)?;
    p.prepare()?;
    let store = AssetStore::open(&p.output)?;
    assert!(store.index().products[&mesh].editor.is_some());
    assert!(!store.dependency_closure(scene())?.contains(&model));
    let binary = p.path.join("dummy-app");
    fs::write(&binary, b"test executable placeholder")?;
    let output = p.path.join("package");
    package(
        &p.output,
        comet_core::GameId::new(),
        scene(),
        &binary,
        &output,
    )?;
    fs::remove_dir_all(p.path.join("assets"))?;
    fs::remove_dir_all(&p.output)?;
    let mut packaged = AssetStore::open(output.join("products"))?;
    assert!(!packaged.index().products.contains_key(&model));
    assert!(
        packaged
            .index()
            .products
            .values()
            .all(|entry| entry.editor.is_none())
    );
    assert!(packaged.index().products.contains_key(&mesh));
    assert!(packaged.index().products.contains_key(&material));
    assert!(!fs::read_to_string(output.join("products/index.json"))?.contains("\"editor\""));
    assert_eq!(packaged.expand_scene(scene())?.entities.len(), 1);
    assert!(matches!(packaged.load(mesh)?.payload, Product::Mesh(_)));
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
    let binary = p.path.join("dummy-app");
    fs::write(&binary, b"test executable placeholder")?;
    let output = p.path.join("package");
    package(
        &p.output,
        comet_core::GameId::new(),
        scene(),
        &binary,
        &output,
    )?;
    let store = AssetStore::open(output.join("products"))?;
    assert_eq!(store.index().products.len(), 12);
    assert!(output.join("package.json").is_file());
    assert!(!output.join("assets").exists());
    assert!(
        package(
            &p.output,
            comet_core::GameId::new(),
            scene(),
            &binary,
            &output
        )
        .is_err()
    );
    assert_eq!(
        fs::read(output.join(if cfg!(windows) {
            "comet-app.exe"
        } else {
            "comet-app"
        }))?,
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
