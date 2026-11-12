use anyhow::Result;
use comet_asset_pipeline::{
    DEFAULT_SCENE, ProjectSettings, SourceMeta, create_prefab, create_project, create_scene,
    prepare,
};
use comet_core::{AssetId, ProjectPaths};
use std::{
    fs,
    path::{Path, PathBuf},
};

type ContentCreator = fn(&ProjectPaths, &Path) -> Result<PathBuf>;
const CONTENT_CREATORS: [(&str, ContentCreator); 2] =
    [("scene", create_scene), ("prefab", create_prefab)];

#[test]
fn asset_build_rejects_missing_assets_before_creating_products_or_locks() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let output = temp.path().join("products");
    assert!(prepare(&project, &output).is_err());
    assert!(!output.exists());
    assert!(fs::read_dir(&project)?.next().is_none());
    Ok(())
}

#[test]
fn asset_build_rejects_overlapping_outputs_before_any_writes() -> Result<()> {
    for relative in ["project/assets", "project/assets/cache", "project", "."] {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        let assets = project.join("assets");
        fs::create_dir_all(&assets)?;
        let source = assets.join("main.scene");
        let bytes = comet_scene::ContentSource::default().to_json()?;
        fs::write(&source, &bytes)?;
        let output = temp.path().join(relative);
        assert!(prepare(&project, &output).is_err(), "{relative}");
        assert_eq!(fs::read(&source)?, bytes);
        assert!(!SourceMeta::path(&source).exists());
        assert!(!project.join(".assetc.lock").exists());
        assert!(!output.join(".build.lock").exists());
        assert!(!output.join("index.json").exists());
        assert_eq!(fs::read_dir(&assets)?.count(), 1);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn asset_build_rejects_escaping_assets_and_aliased_output_overlap_before_any_writes() -> Result<()>
{
    use std::os::unix::fs::symlink;
    for escaped_assets in [false, true] {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        let sources = if escaped_assets {
            temp.path().join("outside")
        } else {
            project.join("assets")
        };
        fs::create_dir_all(&project)?;
        fs::create_dir_all(&sources)?;
        let source = sources.join("main.scene");
        let bytes = comet_scene::ContentSource::default().to_json()?;
        fs::write(&source, &bytes)?;
        let output = temp.path().join("products");
        if escaped_assets {
            symlink(&sources, project.join("assets"))?;
        } else {
            symlink(&sources, &output)?;
        }
        assert!(prepare(&project, &output).is_err());
        assert_eq!(fs::read(&source)?, bytes);
        assert!(!SourceMeta::path(&source).exists());
        assert!(!project.join(".assetc.lock").exists());
        assert!(!output.join(".build.lock").exists());
        assert!(!output.join("index.json").exists());
        assert_eq!(fs::read_dir(&sources)?.count(), 1);
        if escaped_assets {
            assert!(!output.exists());
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn asset_build_preserves_identity_and_labels_through_internal_directory_aliases() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let sources = project.join("source-data");
    fs::create_dir_all(&sources)?;
    symlink(&sources, project.join("assets"))?;
    let alias = temp.path().join("project-alias");
    symlink(&project, &alias)?;
    let cache = temp.path().join("cache");
    fs::create_dir(&cache)?;
    let cache_alias = temp.path().join("cache-alias");
    symlink(&cache, &cache_alias)?;
    let source = sources.join("main.scene");
    let bytes = comet_scene::ContentSource::default().to_json()?;
    fs::write(&source, &bytes)?;
    let first = prepare(&alias, &cache_alias.join("products"))?;
    let metadata = fs::read(SourceMeta::path(&source))?;
    let id = SourceMeta::load(&source)?.id;
    let products = cache.join("products");
    let mut store = comet_asset::AssetStore::open(&products)?;
    assert_eq!(store.expand_scene(id)?.entities.len(), 0);
    let origin = store.index().products[&id].editor.as_ref().unwrap();
    assert_eq!(origin.source, id);
    assert_eq!(origin.label, "main.scene");
    let rebuilt = prepare(&project, &products)?;
    assert_eq!(rebuilt.changed, 0);
    assert_eq!(rebuilt.reused, 1);
    assert_eq!(rebuilt.index_digest, first.index_digest);
    assert_eq!(fs::read(SourceMeta::path(&source))?, metadata);
    assert_eq!(fs::read(&source)?, bytes);
    Ok(())
}

#[test]
fn new_projects_publish_only_authoring_files_and_keep_the_entry_identity_on_rebuild() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let destination = temp.path().join("新项目");
    let paths = create_project(&destination)?;
    assert_eq!(paths.root(), destination.canonicalize()?);
    let mut root_entries: Vec<_> = fs::read_dir(paths.root())?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<_>>()?;
    root_entries.sort();
    assert_eq!(
        root_entries,
        vec![
            std::ffi::OsString::from("assets"),
            std::ffi::OsString::from("project.json")
        ]
    );
    let settings = ProjectSettings::load(&paths)?;
    assert_eq!(settings.version, 2);
    let scene = paths.resolve_asset_path(DEFAULT_SCENE)?;
    let source = comet_scene::ContentSource::from_json(&fs::read(&scene)?)?;
    assert_eq!(source.base.version, comet_scene::scene::SCENE_VERSION);
    assert!(source.base.entities.is_empty());
    let metadata = SourceMeta::path(&scene);
    let reserved = fs::read(&metadata)?;
    let id = SourceMeta::load(&scene)?.id;
    assert!(create_project(&destination).is_err());
    assert_eq!(fs::read(&metadata)?, reserved);
    let products = temp.path().join("products");
    prepare(paths.root(), &products)?;
    let mut store = comet_asset::AssetStore::open(&products)?;
    let revision = store.revision().to_owned();
    assert_eq!(store.expand_scene(id)?, source.base);
    prepare(paths.root(), &products)?;
    assert_eq!(
        comet_asset::AssetStore::open(&products)?.revision(),
        revision
    );
    assert_eq!(SourceMeta::load(&scene)?.id, id);
    assert_eq!(ProjectSettings::load(&paths)?, settings);
    assert_eq!(fs::read(&metadata)?, reserved);
    assert_eq!(ProjectPaths::open(&destination)?.root(), paths.root());
    assert!(!paths.root().join("products").exists());
    let bytes = fs::read(paths.root().join("project.json"))?;
    let replacement = ProjectSettings {
        game_id: comet_core::GameId::new(),
        ..settings.clone()
    };
    assert!(replacement.save(&paths, &mut store).is_err());
    assert_eq!(fs::read(paths.root().join("project.json"))?, bytes);
    let moved = temp.path().join("moved-project");
    fs::rename(paths.root(), &moved)?;
    assert_eq!(
        ProjectSettings::load(&ProjectPaths::open(moved)?)?,
        settings
    );
    Ok(())
}

#[test]
fn invalid_project_versions_fields_and_paths_are_rejected_without_overwriting() -> Result<()> {
    let temp = tempfile::tempdir()?;
    fs::create_dir(temp.path().join("assets"))?;
    let paths = ProjectPaths::open(temp.path())?;
    let configuration = paths.root().join("project.json");
    let id = comet_core::GameId::new();
    for invalid in [
        serde_json::json!({"version":1,"game_id":id}),
        serde_json::json!({"version":1,"game_id":null}),
        serde_json::json!({"version":2}),
        serde_json::json!({"version":2,"game_id":null}),
        serde_json::json!({"version":2,"game_id":"00000000-0000-0000-0000-000000000000"}),
        serde_json::json!({"version":2,"game_id":"bad"}),
        serde_json::json!({"version":2,"game_id":id,"future":true}),
        serde_json::json!({"version":3,"game_id":id}),
    ] {
        let bytes = serde_json::to_vec(&invalid)?;
        fs::write(&configuration, &bytes)?;
        assert!(ProjectSettings::load(&paths).is_err(), "{invalid}");
        assert_eq!(fs::read(&configuration)?, bytes);
    }
    for bytes in [b"invalid json".to_vec(), vec![b' '; 65_537]] {
        fs::write(&configuration, &bytes)?;
        assert!(ProjectSettings::load(&paths).is_err());
        assert_eq!(fs::read(&configuration)?, bytes);
    }
    fs::remove_file(&configuration)?;
    fs::create_dir(&configuration)?;
    assert!(ProjectSettings::load(&paths).is_err());
    fs::remove_dir(&configuration)?;
    #[cfg(unix)]
    {
        let outside = temp.path().join("outside.json");
        fs::write(&outside, b"keep external file")?;
        std::os::unix::fs::symlink(&outside, &configuration)?;
        assert!(ProjectSettings::load(&paths).is_err());
        assert_eq!(fs::read(&outside)?, b"keep external file");
        assert!(
            fs::symlink_metadata(&configuration)?
                .file_type()
                .is_symlink()
        );
    }
    Ok(())
}

#[test]
fn new_project_rejects_existing_directories_files_and_unusable_parents_without_overwriting()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let directory = temp.path().join("directory");
    fs::create_dir(&directory)?;
    fs::write(directory.join("keep.txt"), "keep directory")?;
    let file = temp.path().join("file");
    fs::write(&file, "keep file")?;
    for destination in [&directory, &file, &file.join("child")] {
        assert!(create_project(destination).is_err());
    }
    assert_eq!(fs::read(directory.join("keep.txt"))?, b"keep directory");
    assert_eq!(fs::read(&file)?, b"keep file");
    assert_eq!(fs::read_dir(temp.path())?.count(), 2);
    Ok(())
}

#[cfg(unix)]
#[test]
fn new_project_rejects_existing_and_dangling_symlinks_without_following_them() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let existing = temp.path().join("existing");
    fs::create_dir(&existing)?;
    fs::write(existing.join("keep.txt"), "keep target")?;
    let missing = temp.path().join("missing");
    let link = temp.path().join("link");
    let dangling = temp.path().join("dangling");
    std::os::unix::fs::symlink(&existing, &link)?;
    std::os::unix::fs::symlink(&missing, &dangling)?;
    assert!(create_project(&link).is_err());
    assert!(create_project(&dangling).is_err());
    assert_eq!(fs::read(existing.join("keep.txt"))?, b"keep target");
    assert!(!missing.exists());
    assert!(fs::symlink_metadata(link)?.file_type().is_symlink());
    assert!(fs::symlink_metadata(dangling)?.file_type().is_symlink());
    assert_eq!(fs::read_dir(temp.path())?.count(), 3);
    Ok(())
}

#[test]
fn new_author_files_reuse_reserved_identity_and_reject_existing_or_outside_targets() -> Result<()> {
    for (extension, create) in CONTENT_CREATORS {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("assets/内容"))?;
        let paths = ProjectPaths::open(&project)?;
        let meta = SourceMeta {
            version: 1,
            id: AssetId::new(),
        };
        let relative = PathBuf::from(format!("内容/新文件.{extension}"));
        let metadata = SourceMeta::path(&paths.assets().join(&relative));
        let reserved = serde_json::to_vec(&meta)?;
        fs::write(&metadata, &reserved)?;
        let source = create(&paths, &relative)?;
        assert_eq!(SourceMeta::load(&source)?.id, meta.id);
        assert_eq!(fs::read(&metadata)?, reserved);
        let saved = fs::read(&source)?;
        let content = comet_scene::ContentSource::from_json(&saved)?;
        assert_eq!(content, comet_scene::ContentSource::default());
        assert!(create(&paths, &relative).is_err());
        assert_eq!(fs::read(&source)?, saved);
        assert_eq!(fs::read(&metadata)?, reserved);
        for path in [
            PathBuf::from(format!("../outside.{extension}")),
            PathBuf::from("bad.json"),
            PathBuf::from("内容"),
            PathBuf::from(if extension == "scene" {
                "wrong.prefab"
            } else {
                "wrong.scene"
            }),
            temp.path().join(format!("absolute.{extension}")),
        ] {
            assert!(create(&paths, &path).is_err());
        }
        let products = temp.path().join("products");
        prepare(&project, &products)?;
        let mut store = comet_asset::AssetStore::open(&products)?;
        assert_eq!(store.index().products.len(), 1);
        let product = store.load(meta.id)?;
        match (extension, &product.payload) {
            ("scene", comet_asset_data::product::Product::Scene(built))
            | ("prefab", comet_asset_data::product::Product::Prefab(built)) => {
                assert_eq!(built, &content);
            }
            _ => panic!("created author file has the wrong product kind"),
        }
        assert_eq!(prepare(&project, &products)?.changed, 0);
        assert_eq!(SourceMeta::load(&source)?.id, meta.id);
        assert_eq!(fs::read(&metadata)?, reserved);
        #[cfg(unix)]
        {
            let outside = temp.path().join("outside");
            fs::create_dir(&outside)?;
            std::os::unix::fs::symlink(&outside, paths.assets().join("link"))?;
            let outside_source = PathBuf::from(format!("link/outside.{extension}"));
            assert!(create(&paths, &outside_source).is_err());
            assert!(fs::read_dir(&outside)?.next().is_none());
            let outside_metadata = outside.join("protected.meta");
            fs::write(&outside_metadata, &reserved)?;
            let linked = paths.assets().join(format!("linked.{extension}"));
            std::os::unix::fs::symlink(&outside_metadata, SourceMeta::path(&linked))?;
            assert!(create(&paths, &linked).is_err());
            assert!(!linked.exists());
            assert_eq!(fs::read(&outside_metadata)?, reserved);
        }
    }
    Ok(())
}

#[test]
fn new_author_files_reserve_new_identity_and_reject_bad_reserved_metadata_without_publishing()
-> Result<()> {
    for (extension, create) in CONTENT_CREATORS {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("assets"))?;
        let paths = ProjectPaths::open(&project)?;
        let created = create(&paths, Path::new(&format!("new.{extension}")))?;
        let identity = SourceMeta::load(&created)?.id;
        let bytes = fs::read(SourceMeta::path(&created))?;
        assert!(create(&paths, &created).is_err());
        assert_eq!(SourceMeta::load(&created)?.id, identity);
        assert_eq!(fs::read(SourceMeta::path(&created))?, bytes);

        let relative = PathBuf::from(format!("bad.{extension}"));
        let source = paths.assets().join(&relative);
        let metadata = SourceMeta::path(&source);
        for bad in [
            b"broken metadata".to_vec(),
            serde_json::to_vec(&SourceMeta {
                version: 2,
                id: AssetId::new(),
            })?,
        ] {
            fs::write(&metadata, &bad)?;
            assert!(create(&paths, &relative).is_err());
            assert!(!source.exists());
            assert_eq!(fs::read(&metadata)?, bad);
        }
        let reserved = SourceMeta {
            version: 1,
            id: AssetId::new(),
        };
        let bytes = serde_json::to_vec(&reserved)?;
        fs::write(&metadata, &bytes)?;
        let recovered = create(&paths, &relative)?;
        assert_eq!(SourceMeta::load(&recovered)?.id, reserved.id);
        assert_eq!(fs::read(&metadata)?, bytes);
    }
    Ok(())
}

#[test]
fn source_catalog_uses_built_identities_and_rejects_inconsistent_metadata() -> Result<()> {
    use comet_asset_data::product::AssetKind;
    use comet_asset_pipeline::SourceCatalog;
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("assets/内容"))?;
    let paths = ProjectPaths::open(&project)?;
    let scene = paths.assets().join("内容/shared.scene");
    let prefab = paths.assets().join("内容/shared.prefab");
    let bytes = comet_scene::ContentSource::default().to_json()?;
    fs::write(&scene, &bytes)?;
    fs::write(&prefab, &bytes)?;
    let products = temp.path().join("products");
    prepare(&project, &products)?;
    let store = comet_asset::AssetStore::open(&products)?;
    let catalog = SourceCatalog::read(&paths, store.index())?;
    let scene_id = catalog.of_kind(AssetKind::Scene).next().unwrap().id;
    let prefab_id = catalog.of_kind(AssetKind::Prefab).next().unwrap().id;
    assert_ne!(scene_id, prefab_id);
    assert_eq!(
        catalog.of_kind(AssetKind::Scene).next().unwrap().source,
        Path::new("内容/shared.scene")
    );
    fs::write(paths.assets().join("unbuilt.scene"), &bytes)?;
    assert_eq!(
        SourceCatalog::read(&paths, store.index())?
            .of_kind(AssetKind::Scene)
            .count(),
        1
    );
    let original = fs::read(SourceMeta::path(&prefab))?;
    fs::write(
        SourceMeta::path(&prefab),
        serde_json::to_vec(&SourceMeta {
            version: 1,
            id: scene_id,
        })?,
    )?;
    assert!(SourceCatalog::read(&paths, store.index()).is_err());
    fs::write(
        SourceMeta::path(&prefab),
        serde_json::to_vec(&SourceMeta {
            version: 2,
            id: prefab_id,
        })?,
    )?;
    assert!(SourceMeta::load(&prefab).is_err());
    assert!(SourceCatalog::read(&paths, store.index()).is_err());
    fs::write(SourceMeta::path(&prefab), &original)?;
    assert_eq!(
        catalog.of_kind(AssetKind::Prefab).next().unwrap().id,
        prefab_id
    );
    #[cfg(unix)]
    {
        let outside = temp.path().join("outside");
        fs::create_dir(&outside)?;
        std::os::unix::fs::symlink(&outside, paths.assets().join("link"))?;
        assert!(SourceCatalog::read(&paths, store.index()).is_err());
    }
    Ok(())
}

#[test]
fn populated_prefab_creation_validates_final_identity_and_dependency_kinds_before_reserving_or_publishing()
-> Result<()> {
    use comet_asset_pipeline::create_prefab_with_content;
    use comet_scene::{
        ContentSource, EntityData, PrefabInstance, PrefabResolver, SceneSnapshot, Script,
    };
    let temp = tempfile::tempdir()?;
    let paths = create_project(&temp.path().join("project"))?;
    let base = create_prefab(&paths, Path::new("base.prefab"))?;
    let original_bytes = fs::read(&base)?;
    let original_meta = fs::read(SourceMeta::path(&base))?;
    let script_id = AssetId::new();
    let script_path = paths.assets().join("effect.rhai");
    let script_bytes = b"fn on_start(ctx) {}";
    fs::write(&script_path, script_bytes)?;
    fs::write(
        SourceMeta::path(&script_path),
        serde_json::to_vec(&SourceMeta {
            version: 1,
            id: script_id,
        })?,
    )?;
    let wrapper = create_prefab(&paths, Path::new("wrapper.prefab"))?;
    let mut owner = EntityData::new("script owner");
    let mut script = Script::new(script_id);
    script.enabled = false;
    owner.script = Some(script);
    fs::write(
        &wrapper,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![owner],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    let wrapper_id = SourceMeta::load(&wrapper)?.id;
    let products = temp.path().join("products");
    prepare(paths.root(), &products)?;
    let mut assets = comet_asset::AssetStore::open(&products)?;
    let index = fs::read(products.join("index.json"))?;
    let asset = SourceMeta::load(&base)?.id;
    let entity = EntityData::new("authored root");
    let content = ContentSource {
        base: SceneSnapshot {
            entities: vec![entity.clone()],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: comet_core::InstanceId::new(),
            prefab: asset,
            parent: Some(entity.id),
            overrides: vec![],
        }],
        ..Default::default()
    };
    assert!(
        create_prefab_with_content(&paths, Path::new("base.prefab"), &content, &mut assets)
            .is_err()
    );
    assert_eq!(fs::read(&base)?, original_bytes);
    assert_eq!(fs::read(SourceMeta::path(&base))?, original_meta);

    // The accepted old product must not hide self-use when a reserved identity is reused.
    fs::remove_file(&base)?;
    let error = create_prefab_with_content(&paths, Path::new("base.prefab"), &content, &mut assets)
        .unwrap_err();
    assert!(format!("{error:#}").contains("cycle"));
    assert!(!base.exists());
    assert_eq!(fs::read(SourceMeta::path(&base))?, original_meta);
    assert_eq!(fs::read(products.join("index.json"))?, index);
    let mut invalid = content.clone();
    let mut script = Script::new(asset);
    script.enabled = false;
    invalid.base.entities[0].script = Some(script);
    assert!(
        create_prefab_with_content(&paths, Path::new("invalid.prefab"), &invalid, &mut assets)
            .is_err()
    );
    let invalid_path = paths.assets().join("invalid.prefab");
    assert!(!invalid_path.exists() && !SourceMeta::path(&invalid_path).exists());
    assert_eq!(fs::read(products.join("index.json"))?, index);

    // An old Script identity can recur through an accepted Prefab's resource dependencies.
    fs::remove_file(&script_path)?;
    let reserved = paths.assets().join("reserved.prefab");
    fs::copy(SourceMeta::path(&script_path), SourceMeta::path(&reserved))?;
    let reserved_meta = fs::read(SourceMeta::path(&reserved))?;
    let transitive = ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: comet_core::InstanceId::new(),
            prefab: wrapper_id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    let error = create_prefab_with_content(
        &paths,
        Path::new("reserved.prefab"),
        &transitive,
        &mut assets,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("cycle"));
    assert!(!reserved.exists());
    assert_eq!(fs::read(SourceMeta::path(&reserved))?, reserved_meta);
    assert_eq!(fs::read(products.join("index.json"))?, index);
    fs::write(&script_path, script_bytes)?;
    fs::write(&base, &original_bytes)?;
    let saved =
        create_prefab_with_content(&paths, Path::new("saved.prefab"), &content, &mut assets)?;
    let id = SourceMeta::load(&saved)?.id;
    assert_ne!(id, asset);
    let saved_bytes = fs::read(&saved)?;
    let saved_meta = fs::read(SourceMeta::path(&saved))?;
    let mut expected = content.clone();
    expected.canonicalize();
    assert_eq!(ContentSource::from_json(&saved_bytes)?, expected);
    assert!(
        create_prefab_with_content(
            &paths,
            Path::new("saved.prefab"),
            &ContentSource::default(),
            &mut assets
        )
        .is_err()
    );
    assert_eq!(fs::read(&saved)?, saved_bytes);
    assert_eq!(fs::read(SourceMeta::path(&saved))?, saved_meta);
    prepare(paths.root(), &products)?;
    let mut built = comet_asset::AssetStore::open(&products)?;
    let restored = built.prefab(id)?;
    assert_eq!(restored, expected);
    assert_eq!(
        restored.expand_prefab(id, &mut built)?,
        expected.expand(&mut assets)?
    );
    assert_eq!(fs::read(SourceMeta::path(&base))?, original_meta);
    Ok(())
}
