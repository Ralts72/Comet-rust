use anyhow::Result;
use comet_application::extract;
use comet_asset::AssetStore;
use comet_asset_data::product::{
    INDEX_VERSION, IndexEntry, MaterialData, MeshData, PRODUCT_VERSION, Product, ProductFile,
    ProductIndex,
};
use comet_core::{AssetId, EntityId, InstanceId, digest};
use comet_scene::{ContentSource, EntityData, Model, PrefabInstance, PrefabResolver, Surface};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

struct Products(PathBuf);
impl Drop for Products {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn products() -> Result<(Products, AssetStore, AssetId, AssetId)> {
    let mesh_data: MeshData = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/prefab/assets/mesh.mesh"
    ))?;
    products_with_mesh(mesh_data)
}

fn products_with_mesh(mesh_data: MeshData) -> Result<(Products, AssetStore, AssetId, AssetId)> {
    let products = Products(std::env::temp_dir().join(format!("comet-extract-{}", AssetId::new())));
    let mesh = AssetId::new();
    let material = AssetId::new();
    let mut material_data: MaterialData = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/prefab/assets/material.material"
    ))?;
    material_data.base_color_texture = None;
    let mut index = ProductIndex {
        version: INDEX_VERSION,
        products: BTreeMap::new(),
    };
    for (id, payload) in [
        (mesh, Product::Mesh(mesh_data)),
        (material, Product::Material(material_data)),
    ] {
        let file = ProductFile {
            version: PRODUCT_VERSION,
            id,
            source_digest: digest(b"extraction fixture"),
            payload,
        };
        let bytes = serde_json::to_vec(&file)?;
        let checksum = digest(&bytes);
        let directory = products.0.join("products").join(id.to_string());
        fs::create_dir_all(&directory)?;
        fs::write(directory.join(format!("{checksum}.json")), bytes)?;
        index.products.insert(
            id,
            IndexEntry {
                digest: checksum,
                kind: file.payload.kind(),
                dependencies: file.payload.dependencies(),
                editor: None,
            },
        );
    }
    fs::write(products.0.join("index.json"), serde_json::to_vec(&index)?)?;
    let store = AssetStore::open(&products.0)?;
    Ok((products, store, mesh, material))
}

fn model(name: &str, parent: Option<EntityId>, mesh: AssetId, material: AssetId) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.parent = parent;
    entity.model = Some(Model {
        mesh,
        surface: Surface {
            material,
            roughness: 0.5,
        },
    });
    entity
}

#[test]
fn selection_highlights_each_selected_subtree_including_author_containers_and_prefab_roots()
-> Result<()> {
    let (_products, mut store, mesh, material) = products()?;
    let container = EntityData::new("author container");
    let direct = model("direct child", Some(container.id), mesh, material);
    let outside = model("outside", None, mesh, material);
    let root = EntityData::new("prefab root");
    let first = model("first mesh", Some(root.id), mesh, material);
    let second = model("nested mesh", Some(first.id), mesh, material);
    let instance = InstanceId::new();
    let prefab = ContentSource {
        base: comet_scene::SceneSnapshot {
            entities: vec![second.clone(), root.clone(), first.clone()],
            ..Default::default()
        },
        ..Default::default()
    };
    struct Library(ContentSource);
    impl PrefabResolver for Library {
        fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
            Ok(self.0.clone())
        }
    }
    let source = ContentSource {
        base: comet_scene::SceneSnapshot {
            entities: vec![outside.clone(), direct.clone(), container.clone()],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            id: instance,
            prefab: AssetId::new(),
            parent: Some(container.id),
            overrides: vec![],
        }],
        ..Default::default()
    };
    let snapshot = source.expand(&mut Library(prefab))?;
    let before = snapshot.clone();
    let prefab_root = root.id.in_instance(instance);
    let first = first.id.in_instance(instance);
    let second = second.id.in_instance(instance);
    let highlighted = |draws: Vec<comet_render::Draw>| {
        draws
            .into_iter()
            .filter(|d| d.selected)
            .map(|d| d.entity)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        highlighted(extract(
            &snapshot,
            &mut store,
            &[container.id, first, container.id, EntityId::new()]
        )?),
        BTreeSet::from([direct.id, first, second])
    );
    assert_eq!(
        highlighted(extract(&snapshot, &mut store, &[prefab_root, outside.id])?),
        BTreeSet::from([first, second, outside.id])
    );
    assert_eq!(
        highlighted(extract(&snapshot, &mut store, &[second])?),
        BTreeSet::from([second])
    );
    assert!(highlighted(extract(&snapshot, &mut store, &[])?).is_empty());
    assert!(highlighted(extract(&snapshot, &mut store, &[EntityId::new()])?).is_empty());
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn malformed_hierarchy_fails_before_selection_traversal() -> Result<()> {
    let (_products, mut store, mesh, material) = products()?;
    let mut entity = model("invalid", None, mesh, material);
    entity.parent = Some(entity.id);
    let id = entity.id;
    let snapshot = comet_scene::SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    };
    assert!(extract(&snapshot, &mut store, &[id]).is_err());
    Ok(())
}

#[test]
fn visible_world_geometry_is_extracted_despite_tiny_or_huge_authoring_scales() -> Result<()> {
    use comet_core::math::{Mat3, Vec3};
    for (scale, author_size, world_size) in [
        (1e-3_f32, 1e4_f32, 10_f32),
        (1e20, 1e-20, 1.),
        (-1e20, 1e-20, -1.),
    ] {
        let mesh_data = MeshData {
            version: 1,
            positions: vec![
                [-author_size, -author_size, 0.],
                [author_size, -author_size, 0.],
                [0., author_size, 0.],
            ],
            normals: vec![[0., 0., 1.]; 3],
            uvs: vec![[0.; 2]; 3],
            indices: vec![0, 1, 2],
            joints: None,
            weights: None,
        };
        let (_products, mut store, mesh, material) = products_with_mesh(mesh_data)?;
        let mut entity = model("visible scaled geometry", None, mesh, material);
        entity.transform.scale = [scale; 3];
        let snapshot = comet_scene::SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        };
        let draws = extract(&snapshot, &mut store, &[])?;
        assert_eq!(draws.len(), 1);
        let point = draws[0]
            .transform
            .model()
            .transform_point3(Vec3::X * author_size);
        assert!((point - Vec3::X * world_size).length() < 1e-5);
        assert_eq!(
            draws[0].transform.normal(),
            Mat3::from_diagonal(Vec3::splat(scale.signum()))
        );
    }
    Ok(())
}

#[test]
fn collapsed_models_stay_hidden_but_unreliable_gpu_transforms_report_the_entity() -> Result<()> {
    let (_products, mut store, mesh, material) = products()?;
    let mut entity = model("collapsed geometry", None, mesh, material);
    entity.transform.scale = [0., 1., 1.];
    let mut snapshot = comet_scene::SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    };
    assert!(extract(&snapshot, &mut store, &[])?.is_empty());
    for scale in [[1e-20, 1e20, 1.], [1e-40; 3]] {
        snapshot.entities[0].transform.scale = scale;
        let message = format!(
            "{:#}",
            extract(&snapshot, &mut store, &[])
                .err()
                .expect("an unrepresentable transform cannot silently disappear")
        );
        assert!(message.contains(&snapshot.entities[0].id.to_string()));
        assert!(message.contains("GPU precision"));
    }
    Ok(())
}
