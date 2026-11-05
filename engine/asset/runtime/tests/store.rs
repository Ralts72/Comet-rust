use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::product::{
    EditorProvenance, INDEX_VERSION, IndexEntry, MaterialData, PRODUCT_VERSION, Product,
    ProductFile, ProductIndex,
};
use comet_core::{AssetId, InstanceId, digest};
use comet_scene::{ContentSource, PrefabInstance};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

struct TemporaryProducts(PathBuf);
impl TemporaryProducts {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("comet-store-{}", AssetId::new()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TemporaryProducts {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn material(id: AssetId, roughness: f32) -> ProductFile {
    ProductFile {
        version: PRODUCT_VERSION,
        id,
        source_digest: digest(&roughness.to_le_bytes()),
        payload: Product::Material(MaterialData {
            version: 1,
            base_color: [1.; 4],
            roughness,
            metallic: 0.,
            base_color_texture: None,
        }),
    }
}
fn write_product(root: &Path, index: &mut ProductIndex, file: ProductFile) -> Result<PathBuf> {
    let bytes = serde_json::to_vec(&file)?;
    let hash = digest(&bytes);
    let directory = root.join("products").join(file.id.to_string());
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{hash}.json"));
    fs::write(&path, bytes)?;
    index.products.insert(
        file.id,
        IndexEntry {
            digest: hash,
            kind: file.payload.kind(),
            dependencies: file.payload.dependencies(),
            editor: None,
        },
    );
    Ok(path)
}
fn write_index(root: &Path, index: &ProductIndex) -> Result<()> {
    fs::write(root.join("index.json"), serde_json::to_vec(index)?)?;
    Ok(())
}
fn fixture() -> Result<(TemporaryProducts, ProductIndex, [AssetId; 2])> {
    let directory = TemporaryProducts::new()?;
    let mut index = ProductIndex {
        version: INDEX_VERSION,
        products: BTreeMap::new(),
    };
    let ids = [AssetId::new(), AssetId::new()];
    write_product(directory.path(), &mut index, material(ids[0], 0.2))?;
    write_product(directory.path(), &mut index, material(ids[1], 0.4))?;
    write_index(directory.path(), &index)?;
    Ok((directory, index, ids))
}

#[test]
fn provenance_refresh_updates_the_index_but_reuses_products_and_rejects_bad_metadata_atomically()
-> Result<()> {
    let (directory, mut index, ids) = fixture()?;
    let mut store = AssetStore::open(directory.path())?;
    let original_index = store.shared_index();
    let products = [store.load(ids[0])?, store.load(ids[1])?];
    index.products.get_mut(&ids[0]).unwrap().editor = Some(EditorProvenance {
        source: ids[0],
        label: "models/source.gltf".into(),
    });
    index.products.get_mut(&ids[1]).unwrap().editor = Some(EditorProvenance {
        source: ids[0],
        label: "子材质🌠".into(),
    });
    write_index(directory.path(), &index)?;
    assert!(store.refresh()?.is_empty());
    let accepted = store.shared_index();
    let revision = store.revision().to_owned();
    assert!(!Arc::ptr_eq(&original_index, &accepted));
    assert!(original_index.products[&ids[0]].editor.is_none());
    for (id, product) in ids.into_iter().zip(&products) {
        assert!(Arc::ptr_eq(product, &store.load(id)?));
    }
    assert!(store.refresh()?.is_empty());
    assert!(Arc::ptr_eq(&accepted, &store.shared_index()));
    for editor in [
        EditorProvenance {
            source: AssetId::new(),
            label: "missing source".into(),
        },
        EditorProvenance {
            source: ids[0],
            label: "invalid\0label".into(),
        },
        EditorProvenance {
            source: ids[0],
            label: "🌠".repeat(1025),
        },
    ] {
        let mut invalid = index.clone();
        invalid.products.get_mut(&ids[1]).unwrap().editor = Some(editor);
        write_index(directory.path(), &invalid)?;
        assert!(store.refresh().is_err());
        assert_eq!(store.revision(), revision);
        assert!(Arc::ptr_eq(&accepted, &store.shared_index()));
        for (id, product) in ids.into_iter().zip(&products) {
            assert!(Arc::ptr_eq(product, &store.load(id)?));
        }
    }
    write_index(directory.path(), &index)?;
    assert!(store.refresh()?.is_empty());
    assert!(Arc::ptr_eq(&accepted, &store.shared_index()));
    Ok(())
}

#[test]
fn separately_opened_stores_have_distinct_index_identities_for_the_same_revision() -> Result<()> {
    let (directory, _, _) = fixture()?;
    let first = AssetStore::open(directory.path())?;
    let second = AssetStore::open(directory.path())?;
    assert_eq!(first.revision(), second.revision());
    assert_eq!(first.index(), second.index());
    let first_index = first.shared_index();
    let second_index = second.shared_index();
    assert!(!Arc::ptr_eq(&first_index, &second_index));
    assert!(Arc::ptr_eq(&first_index, &first.shared_index()));
    assert_eq!(first_index.as_ref(), first.index());
    Ok(())
}

#[test]
fn audio_worlds_share_pcm_and_reject_wrong_types_and_oversized_refreshes_without_losing_live_clips()
-> Result<()> {
    let (directory, mut index, ids) = fixture()?;
    let clip = AssetId::new();
    let file = ProductFile {
        version: PRODUCT_VERSION,
        id: clip,
        source_digest: digest(b"tiny PCM"),
        payload: Product::Audio(comet_asset_data::AudioData {
            version: comet_asset_data::audio::AUDIO_VERSION,
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0, 1200, -1200, 0].into(),
        }),
    };
    let product_path = write_product(directory.path(), &mut index, file)?;
    write_index(directory.path(), &index)?;
    let mut store = AssetStore::open(directory.path())?;
    let source = |name| {
        let mut entity = comet_scene::EntityData::new(name);
        entity.audio_source = Some(comet_scene::AudioSource::new(clip));
        entity
    };
    let mut snapshot = comet_scene::SceneSnapshot {
        entities: vec![source("first"), source("second")],
        ..Default::default()
    };
    let library = store.audio_sources(&snapshot)?;
    assert_eq!(library.len(), 1);
    let product = store.load(clip)?;
    let Product::Audio(data) = &product.payload else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&library[&clip].samples, &data.samples));
    snapshot.entities[1].audio_source.as_mut().unwrap().clip = ids[0];
    assert!(store.audio_sources(&snapshot).is_err());
    let accepted_index = store.shared_index();
    let bytes = fs::read(&product_path)?;
    fs::File::options()
        .write(true)
        .open(&product_path)?
        .set_len(comet_asset_data::audio::MAX_AUDIO_BYTES as u64 + 1)?;
    let error = store
        .refresh()
        .expect_err("oversized audio must be rejected before JSON parsing");
    assert!(format!("{error:#}").contains("file size budget"));
    assert!(Arc::ptr_eq(&accepted_index, &store.shared_index()));
    assert!(Arc::ptr_eq(&product, &store.load(clip)?));
    assert_eq!(library[&clip].samples.as_ref(), &[0, 1200, -1200, 0]);
    fs::write(product_path, bytes)?;
    assert!(store.refresh()?.is_empty());
    Ok(())
}

#[test]
fn unchanged_refresh_preserves_index_and_resident_products_while_eviction_releases_only_the_store()
-> Result<()> {
    let (directory, _, ids) = fixture()?;
    let mut store = AssetStore::open(directory.path())?;
    let index = store.shared_index();
    let product = store.load(ids[0])?;
    assert_eq!(store.resident_count(), 1);
    assert!(store.refresh()?.is_empty());
    assert!(Arc::ptr_eq(&index, &store.shared_index()));
    assert!(Arc::ptr_eq(&product, &store.load(ids[0])?));
    assert_eq!(store.resident_count(), 1);
    store.evict(ids[0]);
    assert_eq!(store.resident_count(), 0);
    assert_eq!(product.as_ref(), &material(ids[0], 0.2));
    assert!(Arc::ptr_eq(&index, &store.shared_index()));
    let reloaded = store.load(ids[0])?;
    assert!(!Arc::ptr_eq(&product, &reloaded));
    assert_eq!(product, reloaded);
    Ok(())
}

#[test]
fn accepted_index_bytes_publish_new_identity_and_keep_previous_consumers_readable() -> Result<()> {
    let (directory, mut index, ids) = fixture()?;
    let mut store = AssetStore::open(directory.path())?;
    let previous_index = store.shared_index();
    let previous_product = store.load(ids[0])?;
    let unchanged_product = store.load(ids[1])?;
    write_product(directory.path(), &mut index, material(ids[0], 0.8))?;
    write_index(directory.path(), &index)?;
    assert_eq!(store.refresh()?, BTreeSet::from([ids[0]]));
    let accepted_index = store.shared_index();
    let accepted_product = store.load(ids[0])?;
    assert!(!Arc::ptr_eq(&previous_index, &accepted_index));
    assert!(!Arc::ptr_eq(&previous_product, &accepted_product));
    assert_ne!(
        previous_index.products[&ids[0]],
        accepted_index.products[&ids[0]]
    );
    assert_eq!(previous_product.as_ref(), &material(ids[0], 0.2));
    assert_eq!(accepted_product.as_ref(), &material(ids[0], 0.8));
    assert!(Arc::ptr_eq(&unchanged_product, &store.load(ids[1])?));
    let revision = store.revision().to_owned();
    fs::write(
        directory.path().join("index.json"),
        serde_json::to_vec_pretty(&index)?,
    )?;
    assert!(store.refresh()?.is_empty());
    assert_ne!(store.revision(), revision);
    let formatted_index = store.shared_index();
    assert_eq!(formatted_index, accepted_index);
    assert!(!Arc::ptr_eq(&formatted_index, &accepted_index));
    assert!(Arc::ptr_eq(&accepted_product, &store.load(ids[0])?));
    assert!(Arc::ptr_eq(&unchanged_product, &store.load(ids[1])?));
    drop(store);
    assert_eq!(previous_product.as_ref(), &material(ids[0], 0.2));
    assert_eq!(accepted_product.as_ref(), &material(ids[0], 0.8));
    assert_eq!(previous_index.products.len(), 2);
    Ok(())
}

#[test]
fn rejected_refreshes_revalidate_even_unchanged_indices_and_preserve_accepted_state() -> Result<()>
{
    let (directory, mut index, ids) = fixture()?;
    let mut store = AssetStore::open(directory.path())?;
    let accepted_index = store.shared_index();
    let accepted_product = store.load(ids[0])?;
    let revision = store.revision().to_owned();
    let assert_accepted = |store: &mut AssetStore| -> Result<()> {
        assert!(Arc::ptr_eq(&accepted_index, &store.shared_index()));
        assert_eq!(store.revision(), revision);
        assert_eq!(store.resident_count(), 1);
        assert!(Arc::ptr_eq(&accepted_product, &store.load(ids[0])?));
        Ok(())
    };
    let product_path = directory
        .path()
        .join("products")
        .join(ids[0].to_string())
        .join(format!("{}.json", index.products[&ids[0]].digest));
    let bytes = fs::read(&product_path)?;
    fs::remove_file(&product_path)?;
    assert!(store.refresh().is_err());
    assert_accepted(&mut store)?;
    fs::write(&product_path, b"corrupt bytes with unchanged index")?;
    assert!(store.refresh().is_err());
    assert_accepted(&mut store)?;
    fs::write(&product_path, bytes)?;
    let mut unsupported = index.clone();
    unsupported.version += 1;
    write_index(directory.path(), &unsupported)?;
    assert!(store.refresh().is_err());
    assert_accepted(&mut store)?;
    let prefab = AssetId::new();
    write_product(
        directory.path(),
        &mut index,
        ProductFile {
            version: PRODUCT_VERSION,
            id: prefab,
            source_digest: digest(b"self-referencing prefab"),
            payload: Product::Prefab(ContentSource {
                instances: vec![PrefabInstance {
                    id: InstanceId::new(),
                    prefab,
                    parent: None,
                    overrides: vec![],
                }],
                ..Default::default()
            }),
        },
    )?;
    write_index(directory.path(), &index)?;
    assert!(store.refresh().is_err());
    assert_accepted(&mut store)?;
    write_index(directory.path(), &accepted_index)?;
    assert!(store.refresh()?.is_empty());
    assert_accepted(&mut store)?;
    Ok(())
}
