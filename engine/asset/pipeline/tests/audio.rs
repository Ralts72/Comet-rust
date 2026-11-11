use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::{
    NavigationData,
    audio::MAX_AUDIO_SAMPLES,
    product::{AssetKind, INDEX_VERSION, PRODUCT_VERSION, Product, ProductIndex},
};
use comet_asset_pipeline::{
    DEFAULT_SCENE, SourceCatalog, SourceMeta, create_project, package, prepare,
};
use comet_core::{AssetId, InstanceId, ProjectPaths};
use comet_scene::{
    AudioSource, ContentSource, EntityData, FieldOverride, PrefabInstance, SceneSnapshot, schema,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

// A tiny independent RIFF fixture, rather than a copy of the demonstration recordings.
fn wav(rate: u32, channels: u16, bits: u16, tag: u16, samples: &[u8]) -> Vec<u8> {
    let block = channels.saturating_mul(bits / 8);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&tag.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&rate.saturating_mul(block as u32).to_le_bytes());
    bytes.extend_from_slice(&block.to_le_bytes());
    bytes.extend_from_slice(&bits.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    bytes.extend_from_slice(samples);
    bytes
}
fn pcm(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    wav(
        rate,
        channels,
        16,
        1,
        &samples
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
    )
}
struct Fixture {
    temp: tempfile::TempDir,
    project: PathBuf,
    products: PathBuf,
    clip: AssetId,
}
impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        let products = temp.path().join("products");
        create_project(&project)?;
        let source = project.join("assets/tone.wav");
        fs::write(&source, pcm(44_100, 1, &[i16::MIN, -1, 0, 1, i16::MAX]))?;
        let clip = AssetId::new();
        fs::write(
            SourceMeta::path(&source),
            serde_json::to_vec_pretty(&SourceMeta {
                version: 1,
                id: clip,
            })?,
        )?;
        prepare(&project, &products)?;
        Ok(Self {
            temp,
            project,
            products,
            clip,
        })
    }
    fn source(&self) -> PathBuf {
        self.project.join("assets/tone.wav")
    }
}
fn write_content(path: &Path, source: &ContentSource) -> Result<()> {
    fs::write(path, source.to_json()?)?;
    Ok(())
}

#[test]
fn pcm16_import_preserves_samples_reserved_metadata_identity_and_catalog_through_rebakes_and_rename()
-> Result<()> {
    let f = Fixture::new()?;
    let metadata = fs::read(SourceMeta::path(&f.source()))?;
    let mut store = AssetStore::open(&f.products)?;
    assert_eq!(store.index().version, INDEX_VERSION);
    let file = store.load(f.clip)?;
    assert_eq!(file.version, PRODUCT_VERSION);
    let Product::Audio(audio) = &file.payload else {
        panic!("audio")
    };
    assert_eq!((audio.sample_rate, audio.channels), (44_100, 1));
    assert_eq!(&*audio.samples, &[i16::MIN, -1, 0, 1, i16::MAX]);
    assert!(Arc::ptr_eq(&audio.samples, &audio.clone().samples));
    let old_digest = store.index().products[&f.clip].digest.clone();
    assert_eq!(prepare(&f.project, &f.products)?.changed, 0);
    fs::write(f.source(), pcm(48_000, 2, &[1, -1, i16::MAX, i16::MIN]))?;
    assert_eq!(prepare(&f.project, &f.products)?.changed, 1);
    store.refresh()?;
    assert_ne!(store.index().products[&f.clip].digest, old_digest);
    assert_eq!(fs::read(SourceMeta::path(&f.source()))?, metadata);
    let Product::Audio(audio) = &store.load(f.clip)?.payload else {
        panic!("audio")
    };
    assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
    assert_eq!(&*audio.samples, &[1, -1, i16::MAX, i16::MIN]);
    let renamed = f.project.join("assets/renamed.wav");
    fs::rename(f.source(), &renamed)?;
    fs::rename(SourceMeta::path(&f.source()), SourceMeta::path(&renamed))?;
    assert_eq!(prepare(&f.project, &f.products)?.changed, 0);
    store.refresh()?;
    let paths = ProjectPaths::open(&f.project)?;
    let catalog = SourceCatalog::read(&paths, store.index())?;
    let sources: Vec<_> = catalog.of_kind(AssetKind::Audio).collect();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].id, f.clip);
    assert_eq!(sources[0].source, PathBuf::from("renamed.wav"));
    assert_eq!(fs::read(SourceMeta::path(&renamed))?, metadata);
    Ok(())
}

#[test]
fn unsupported_truncated_and_oversized_wavs_leave_the_accepted_index_and_metadata_intact()
-> Result<()> {
    let f = Fixture::new()?;
    let index = fs::read(f.products.join("index.json"))?;
    let metadata = fs::read(SourceMeta::path(&f.source()))?;
    let valid = fs::read(f.source())?;
    let mut count_overflow = pcm(8_000, 1, &[0]);
    count_overflow[40..44].copy_from_slice(&((MAX_AUDIO_SAMPLES as u32 + 1) * 2).to_le_bytes());
    let mut truncated = pcm(8_000, 1, &[0, 1]);
    truncated.pop();
    for bytes in [
        b"not a wave".to_vec(),
        wav(8_000, 1, 8, 1, &[0]),
        wav(8_000, 1, 24, 1, &[0; 3]),
        wav(8_000, 1, 32, 3, &[0; 4]),
        pcm(8_000, 0, &[0]),
        pcm(8_000, 3, &[0, 1, 2]),
        pcm(7_999, 1, &[0]),
        pcm(192_001, 1, &[0]),
        pcm(8_000, 1, &[]),
        pcm(8_000, 2, &[0]),
        count_overflow,
        truncated,
    ] {
        fs::write(f.source(), bytes)?;
        assert!(prepare(&f.project, &f.products).is_err());
        assert_eq!(fs::read(f.products.join("index.json"))?, index);
        assert_eq!(fs::read(SourceMeta::path(&f.source()))?, metadata);
    }
    fs::write(f.source(), &valid)?;
    fs::OpenOptions::new()
        .write(true)
        .open(f.source())?
        .set_len((MAX_AUDIO_SAMPLES * 2 + 64 * 1024 + 1) as u64)?;
    let error = prepare(&f.project, &f.products).unwrap_err();
    assert!(format!("{error:#}").contains("byte budget"));
    assert_eq!(fs::read(f.products.join("index.json"))?, index);
    fs::write(f.source(), valid)?;
    let bad = f.project.join("assets/unaccepted.wav");
    fs::write(&bad, pcm(8_000, 1, &[]))?;
    assert!(prepare(&f.project, &f.products).is_err());
    assert!(!SourceMeta::path(&bad).exists());
    let mut store = AssetStore::open(&f.products)?;
    assert_eq!(store.load(f.clip)?.payload.kind(), AssetKind::Audio);
    Ok(())
}

#[test]
fn prefab_clip_overrides_package_their_dependencies_and_load_without_any_authoring_directory()
-> Result<()> {
    let f = Fixture::new()?;
    let override_path = f.project.join("assets/override.wav");
    let unused_path = f.project.join("assets/unused.wav");
    fs::write(&override_path, pcm(8_000, 2, &[1, 2, 3, 4]))?;
    fs::write(&unused_path, pcm(8_000, 1, &[9]))?;
    prepare(&f.project, &f.products)?;
    let override_clip = SourceMeta::load(&override_path)?.id;
    let unused = SourceMeta::load(&unused_path)?.id;
    let mut owner = EntityData::new("audio source");
    owner.audio_source = Some(AudioSource {
        gain: 0.5,
        looping: true,
        ..AudioSource::new(f.clip)
    });
    let owner_id = owner.id;
    let prefab_path = f.project.join("assets/voice.prefab");
    write_content(
        &prefab_path,
        &ContentSource {
            base: SceneSnapshot {
                version: 7,
                entities: vec![owner],
            },
            ..Default::default()
        },
    )?;
    prepare(&f.project, &f.products)?;
    let prefab = SourceMeta::load(&prefab_path)?.id;
    let scene_path = f.project.join("assets").join(DEFAULT_SCENE);
    let scene = SourceMeta::load(&scene_path)?.id;
    write_content(
        &scene_path,
        &ContentSource {
            base: SceneSnapshot {
                version: 7,
                ..Default::default()
            },
            instances: vec![PrefabInstance {
                translation: [0.; 3],
                id: InstanceId::new(),
                prefab,
                parent: None,
                overrides: vec![FieldOverride {
                    entity: owner_id,
                    field: schema::AUDIO_CLIP,
                    value: schema::FieldValue::Asset(override_clip),
                }],
            }],
            ..Default::default()
        },
    )?;
    prepare(&f.project, &f.products)?;
    let output = f.temp.path().join("export");
    package(
        &f.products,
        comet_core::GameId::new(),
        scene,
        &std::env::current_exe()?,
        &output,
    )?;
    let moved = f.temp.path().join("elsewhere/game");
    fs::create_dir_all(moved.parent().unwrap())?;
    fs::rename(output, &moved)?;
    fs::remove_dir_all(&f.project)?;
    fs::remove_dir_all(&f.products)?;
    let mut store = AssetStore::open(moved.join("products"))?;
    assert!(
        store
            .index()
            .products
            .values()
            .all(|entry| entry.editor.is_none())
    );
    assert!(!store.index().products.contains_key(&unused));
    assert_eq!(store.dependency_closure(scene)?.len(), 4);
    let snapshot = store.expand_scene(scene)?;
    assert_eq!(
        snapshot.version, 7,
        "2D declarations retain their author protocol"
    );
    assert_eq!(
        snapshot.entities[0].audio_source.as_ref().unwrap().clip,
        override_clip
    );
    let Product::Audio(audio) = &store.load(override_clip)?.payload else {
        panic!("audio")
    };
    assert_eq!(&*audio.samples, &[1, 2, 3, 4]);
    Ok(())
}

#[test]
fn old_indexes_rebuild_without_changing_author_ids_but_cannot_smuggle_audio() -> Result<()> {
    let f = Fixture::new()?;
    fs::write(f.project.join("assets/cached.rhai"), "let cached = true;")?;
    fs::write(
        f.project.join("assets/cached.nav"),
        serde_json::to_vec(&NavigationData {
            version: 1,
            input_digest: "0".repeat(64),
            vertices: vec![[-1., 0., -1.], [1., 0., -1.], [0., 0., 1.]],
            polygons: vec![vec![0, 1, 2]],
        })?,
    )?;
    prepare(&f.project, &f.products)?;
    let index_path = f.products.join("index.json");
    let current: ProductIndex = serde_json::from_slice(&fs::read(&index_path)?)?;
    let metadata = fs::read(SourceMeta::path(&f.source()))?;
    for version in 1..=4 {
        let mut legacy = current.clone();
        legacy.version = version;
        legacy.products.retain(|_, entry| {
            entry.kind != AssetKind::Audio
                && (version >= 2 || entry.kind != AssetKind::Script)
                && (version >= 4 || entry.kind != AssetKind::Navigation)
        });
        if version < 3 {
            for entry in legacy.products.values_mut() {
                entry.editor = None;
            }
        }
        fs::write(&index_path, serde_json::to_vec(&legacy)?)?;
        assert!(AssetStore::open(&f.products).is_err());
        prepare(&f.project, &f.products)?;
        let mut store = AssetStore::open(&f.products)?;
        assert_eq!(store.index().version, INDEX_VERSION);
        assert_eq!(store.load(f.clip)?.version, PRODUCT_VERSION);
        assert_eq!(fs::read(SourceMeta::path(&f.source()))?, metadata);
        legacy
            .products
            .insert(f.clip, current.products[&f.clip].clone());
        if version < 3 {
            legacy.products.get_mut(&f.clip).unwrap().editor = None;
        }
        let bad = serde_json::to_vec(&legacy)?;
        fs::write(&index_path, &bad)?;
        let error = prepare(&f.project, &f.products).unwrap_err();
        assert!(format!("{error:#}").contains("newer audio asset kind"));
        assert_eq!(fs::read(&index_path)?, bad);
    }
    Ok(())
}
