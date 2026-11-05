use super::{SceneExtractor, extract};
use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::product::{
    INDEX_VERSION, IndexEntry, MaterialData, MeshData, PRODUCT_VERSION, Product, ProductFile,
    ProductIndex,
};
use comet_core::{AssetId, EntityId, digest, math::Vec3};
use comet_render::Draw;
use comet_scene::{EntityData, Model, SceneSnapshot, Surface};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::Arc,
};
mod skin;

struct Products {
    root: PathBuf,
    mesh: AssetId,
    material: AssetId,
    index: ProductIndex,
}

impl Drop for Products {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Products {
    fn new() -> Result<Self> {
        Self::with_ids(AssetId::new(), AssetId::new())
    }

    fn with_ids(mesh: AssetId, material: AssetId) -> Result<Self> {
        let mut products = Self {
            root: std::env::temp_dir().join(format!("comet-extraction-cache-{}", AssetId::new())),
            mesh,
            material,
            index: ProductIndex {
                version: INDEX_VERSION,
                products: BTreeMap::new(),
            },
        };
        let mesh_data: MeshData = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/prefab/assets/mesh.mesh"
        ))?;
        let mut material_data: MaterialData = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/prefab/assets/material.material"
        ))?;
        material_data.base_color_texture = None;
        products.write(mesh, Product::Mesh(mesh_data))?;
        products.write(material, Product::Material(material_data))?;
        products.publish()?;
        Ok(products)
    }

    fn write(&mut self, id: AssetId, payload: Product) -> Result<()> {
        let product = ProductFile {
            version: PRODUCT_VERSION,
            id,
            source_digest: digest(b"extraction cache fixture"),
            payload,
        };
        let bytes = serde_json::to_vec(&product)?;
        let checksum = digest(&bytes);
        let directory = self.root.join("products").join(id.to_string());
        fs::create_dir_all(&directory)?;
        fs::write(directory.join(format!("{checksum}.json")), bytes)?;
        self.index.products.insert(
            id,
            IndexEntry {
                digest: checksum,
                kind: product.payload.kind(),
                dependencies: product.payload.dependencies(),
                editor: None,
            },
        );
        Ok(())
    }

    fn publish(&self) -> Result<()> {
        fs::write(
            self.root.join("index.json"),
            serde_json::to_vec(&self.index)?,
        )?;
        Ok(())
    }

    fn path(&self, id: AssetId) -> PathBuf {
        self.root
            .join("products")
            .join(id.to_string())
            .join(format!("{}.json", self.index.products[&id].digest))
    }

    fn store(&self) -> Result<AssetStore> {
        AssetStore::open(&self.root)
    }

    fn model(&self, name: &str, parent: Option<EntityId>) -> EntityData {
        let mut entity = EntityData::new(name);
        entity.parent = parent;
        entity.model = Some(Model {
            mesh: self.mesh,
            surface: Surface {
                material: self.material,
                roughness: 0.5,
            },
        });
        entity
    }
}

fn snapshot(entities: Vec<EntityData>) -> Arc<SceneSnapshot> {
    Arc::new(SceneSnapshot {
        entities,
        ..Default::default()
    })
}

fn selected(draws: &[Draw]) -> BTreeSet<EntityId> {
    draws
        .iter()
        .filter(|draw| draw.selected)
        .map(|draw| draw.entity)
        .collect()
}

#[test]
fn identical_keys_reuse_draws_without_reloading_evicted_products() -> Result<()> {
    let products = Products::new()?;
    let mut store = products.store()?;
    let scene = snapshot(vec![products.model("model", None)]);
    let mut extractor = SceneExtractor::default();
    let first = extractor.extract(&scene, &mut store, &[])?;
    assert_eq!(store.resident_count(), 2);
    store.evict(products.mesh);
    store.evict(products.material);
    assert_eq!(store.resident_count(), 0);
    let again = extractor.extract(&scene, &mut store, &[])?;
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(store.resident_count(), 0);
    assert!(matches!(first[0].mesh.payload, Product::Mesh(_)));
    let index = store.shared_index();
    assert!(store.refresh()?.is_empty());
    assert!(Arc::ptr_eq(&index, &store.shared_index()));
    let refreshed = extractor.extract(&scene, &mut store, &[])?;
    assert!(Arc::ptr_eq(&first, &refreshed));
    assert_eq!(store.resident_count(), 0);
    Ok(())
}

#[test]
fn author_instance_references_never_enter_extraction_or_replace_the_last_draw_batch() -> Result<()>
{
    let products = Products::new()?;
    let mut store = products.store()?;
    let scene = snapshot(vec![products.model("accepted", None)]);
    let mut extractor = SceneExtractor::default();
    let accepted = extractor.extract(&scene, &mut store, &[])?;
    let mut invalid = (*scene).clone();
    invalid.entities[0].links.target = Some(comet_scene::EntityRef::in_instance(
        comet_core::InstanceId::new(),
        EntityId::new(),
    ));
    invalid.validate()?;
    let error = extractor
        .extract(&Arc::new(invalid), &mut store, &[])
        .err()
        .expect("unresolved author reference must be rejected");
    assert!(error.to_string().contains("unresolved prefab instance"));
    assert!(error.downcast_ref::<comet_scene::UnboundRig>().is_none());
    assert!(Arc::ptr_eq(
        &accepted,
        &extractor.extract(&scene, &mut store, &[])?
    ));
    Ok(())
}

#[test]
fn changing_selection_reuses_geometry_and_preserves_inflight_highlights() -> Result<()> {
    let products = Products::new()?;
    let mut store = products.store()?;
    let container = EntityData::new("container");
    let child = products.model("child", Some(container.id));
    let descendant = products.model("descendant", Some(child.id));
    let outside = products.model("outside", None);
    let ids = (container.id, child.id, descendant.id, outside.id);
    let scene = snapshot(vec![outside, descendant, container, child]);
    let mut extractor = SceneExtractor::default();
    let first = extractor.extract(&scene, &mut store, &[])?;
    store.evict(products.mesh);
    store.evict(products.material);
    let group = extractor.extract(&scene, &mut store, &[ids.0])?;
    assert!(!Arc::ptr_eq(&first, &group));
    assert_eq!(selected(&group), BTreeSet::from([ids.1, ids.2]));
    assert!(selected(&first).is_empty());
    assert_eq!(store.resident_count(), 0);
    for (old, new) in first.iter().zip(group.iter()) {
        assert_eq!(old.entity, new.entity);
        assert_eq!(old.transform, new.transform);
        assert!(Arc::ptr_eq(&old.mesh, &new.mesh));
        assert!(Arc::ptr_eq(&old.material, &new.material));
    }
    assert!(Arc::ptr_eq(
        &group,
        &extractor.extract(&scene, &mut store, &[ids.0])?
    ));
    let leaf = extractor.extract(&scene, &mut store, &[ids.2])?;
    assert_eq!(selected(&leaf), BTreeSet::from([ids.2]));
    assert_eq!(selected(&group), BTreeSet::from([ids.1, ids.2]));
    assert!(selected(&extractor.extract(&scene, &mut store, &[EntityId::new()])?).is_empty());
    assert!(selected(&extractor.extract(&scene, &mut store, &[])?).is_empty());
    assert_eq!(store.resident_count(), 0);
    Ok(())
}

#[test]
fn scene_and_product_versions_publish_new_draws_while_older_products_remain_readable() -> Result<()>
{
    let mut products = Products::new()?;
    let mut store = products.store()?;
    let scene = snapshot(vec![products.model("model", None)]);
    let mut extractor = SceneExtractor::default();
    let first = extractor.extract(&scene, &mut store, &[])?;
    let equal_copy = Arc::new(scene.as_ref().clone());
    let equal_draws = extractor.extract(&equal_copy, &mut store, &[])?;
    assert!(!Arc::ptr_eq(&first, &equal_draws));
    let mut moved = scene.as_ref().clone();
    moved.entities[0].transform.translation = [2., 0., 0.];
    let moved = Arc::new(moved);
    let moved_draws = extractor.extract(&moved, &mut store, &[])?;
    assert_eq!(
        first[0].transform.model().transform_point3(Vec3::ZERO),
        Vec3::ZERO
    );
    assert_eq!(
        moved_draws[0]
            .transform
            .model()
            .transform_point3(Vec3::ZERO),
        Vec3::new(2., 0., 0.)
    );
    let old_index = store.shared_index();
    let Product::Material(old_material) = &first[0].material.payload else {
        unreachable!()
    };
    let mut new_material = old_material.clone();
    new_material.base_color = [0.1, 0.6, 0.9, 1.];
    products.write(products.material, Product::Material(new_material.clone()))?;
    products.publish()?;
    assert_eq!(store.refresh()?, BTreeSet::from([products.material]));
    assert!(!Arc::ptr_eq(&old_index, &store.shared_index()));
    let updated = extractor.extract(&moved, &mut store, &[])?;
    assert!(!Arc::ptr_eq(&moved_draws, &updated));
    assert!(Arc::ptr_eq(&first[0].mesh, &updated[0].mesh));
    assert!(!Arc::ptr_eq(&first[0].material, &updated[0].material));
    assert_eq!(updated[0].material.payload, Product::Material(new_material));
    assert_eq!(
        first[0].material.payload,
        Product::Material(old_material.clone())
    );
    Ok(())
}

#[test]
fn a_new_store_with_an_identical_index_cannot_reuse_another_stores_draws() -> Result<()> {
    let first_products = Products::new()?;
    let second_products = Products::with_ids(first_products.mesh, first_products.material)?;
    let mut first_store = first_products.store()?;
    let mut second_store = second_products.store()?;
    assert_eq!(first_store.revision(), second_store.revision());
    assert!(!Arc::ptr_eq(
        &first_store.shared_index(),
        &second_store.shared_index()
    ));
    let scene = snapshot(vec![first_products.model("model", None)]);
    let mut extractor = SceneExtractor::default();
    let first = extractor.extract(&scene, &mut first_store, &[])?;
    fs::remove_file(second_products.path(second_products.mesh))?;
    assert!(extractor.extract(&scene, &mut second_store, &[]).is_err());
    assert!(Arc::ptr_eq(
        &first,
        &extractor.extract(&scene, &mut first_store, &[])?
    ));
    fs::copy(
        first_products.path(first_products.mesh),
        second_products.path(second_products.mesh),
    )?;
    let retried = extractor.extract(&scene, &mut second_store, &[])?;
    assert!(!Arc::ptr_eq(&first, &retried));
    assert!(!Arc::ptr_eq(&first[0].mesh, &retried[0].mesh));
    assert_eq!(first[0].mesh.payload, retried[0].mesh.payload);
    Ok(())
}

#[test]
fn rejected_scene_transform_and_asset_candidates_leave_the_last_successful_batch_intact()
-> Result<()> {
    let products = Products::new()?;
    let mut store = products.store()?;
    let scene = snapshot(vec![products.model("model", None)]);
    let mut extractor = SceneExtractor::default();
    let accepted = extractor.extract(&scene, &mut store, &[])?;
    let mut malformed = scene.as_ref().clone();
    malformed.entities[0].parent = Some(malformed.entities[0].id);
    let mut bad_transform = scene.as_ref().clone();
    bad_transform.entities[0].transform.scale = [1e-40; 3];
    let mut bad_asset = scene.as_ref().clone();
    bad_asset.entities[0].model.as_mut().unwrap().mesh = AssetId::new();
    for candidate in [malformed, bad_transform, bad_asset] {
        assert!(
            extractor
                .extract(&Arc::new(candidate), &mut store, &[])
                .is_err()
        );
        assert!(Arc::ptr_eq(
            &accepted,
            &extractor.extract(&scene, &mut store, &[])?
        ));
        assert_eq!(accepted[0].entity, scene.entities[0].id);
        assert!(!accepted[0].selected);
    }
    Ok(())
}

#[test]
#[ignore = "CPU extraction timing diagnostic; run explicitly to write target/verification evidence"]
fn extraction_cache_cpu_profile() -> Result<()> {
    use std::{hint::black_box, time::Instant};
    fn measure(iterations: usize, mut call: impl FnMut() -> Result<()>) -> Result<f64> {
        let start = Instant::now();
        for _ in 0..iterations {
            call()?;
        }
        Ok(start.elapsed().as_secs_f64() * 1000. / iterations as f64)
    }
    let products = Products::new()?;
    let mut samples = vec![];
    for models in [1_000, 10_000] {
        let left = EntityData::new("left group");
        let right = EntityData::new("right group");
        let group_ids = (left.id, right.id);
        let mut entities = vec![left, right];
        for i in 0..models {
            let mut entity = products.model(
                "model",
                Some(if i % 2 == 0 { group_ids.0 } else { group_ids.1 }),
            );
            entity.transform.translation = [(i % 100) as f32, (i / 100) as f32, 0.];
            entities.push(entity);
        }
        let scene = snapshot(entities);
        let mut store = products.store()?;
        assert_eq!(extract(&scene, &mut store, &[])?.len(), models);
        let mut extractor = SceneExtractor::default();
        assert_eq!(extractor.extract(&scene, &mut store, &[])?.len(), models);
        let full_iterations = 10;
        let full_ms = measure(full_iterations, || {
            black_box(extract(
                black_box(&scene),
                black_box(&mut store),
                black_box(&[]),
            )?);
            Ok(())
        })?;
        let hit_iterations = 10_000;
        let hit_ms = measure(hit_iterations, || {
            black_box(extractor.extract(
                black_box(&scene),
                black_box(&mut store),
                black_box(&[]),
            )?);
            Ok(())
        })?;
        let selection_iterations = 20;
        let mut alternate = false;
        let selection_ms = measure(selection_iterations, || {
            alternate = !alternate;
            let selection = [if alternate { group_ids.0 } else { group_ids.1 }];
            black_box(extractor.extract(
                black_box(&scene),
                black_box(&mut store),
                black_box(&selection),
            )?);
            Ok(())
        })?;
        samples.push(serde_json::json!({
            "models":models, "entities":scene.entities.len(),
            "full_extract":{"iterations":full_iterations,"milliseconds_per_call":full_ms},
            "cache_hit":{"iterations":hit_iterations,"milliseconds_per_call":hit_ms},
            "selection_only":{"iterations":selection_iterations,"milliseconds_per_call":selection_ms},
        }));
    }
    let report = serde_json::json!({
        "scope":"CPU scene extraction only; warmed product store; no GPU, UI, or overall frame-rate measurement",
        "debug_assertions":cfg!(debug_assertions),
        "samples":samples,
    });
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let output = root.join("target/verification/extraction-cache-profile.json");
    fs::create_dir_all(output.parent().unwrap())?;
    fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", report);
    Ok(())
}
