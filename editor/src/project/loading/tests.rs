use super::*;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn empty_project(root: &Path) -> Result<ProjectPaths> {
    fs::create_dir_all(root.join("assets"))?;
    fs::write(
        root.join("project.json"),
        serde_json::to_vec(&comet_asset_pipeline::ProjectSettings::for_new_project())?,
    )?;
    ProjectPaths::open(root)
}

fn fixture(root: &Path) -> Result<()> {
    copy_fixture(root, "gltf", &[])
}

fn prefab_fixture(root: &Path) -> Result<()> {
    copy_fixture(root, "prefab", &["main.scene", "main.scene.meta"])
}

fn copy_fixture(root: &Path, name: &str, excluded: &[&str]) -> Result<()> {
    fs::create_dir_all(root.join("assets"))?;
    fs::write(
        root.join("project.json"),
        serde_json::to_vec(&comet_asset_pipeline::ProjectSettings::for_new_project())?,
    )?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name)
        .join("assets");
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() != ".assetc.lock"
            && !excluded.iter().any(|name| entry.file_name() == *name)
        {
            fs::copy(entry.path(), root.join("assets").join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn wait(load: &mut ProjectLoad) -> std::result::Result<ProjectCandidate, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(result) = load.poll() {
            return result;
        }
        assert!(Instant::now() < deadline, "project load did not finish");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn project_caches_keep_the_initial_cli_location_and_never_nest_after_switches() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let first = empty_project(&temp.path().join("first"))?;
    let second = empty_project(&temp.path().join("second"))?;
    let third = empty_project(&temp.path().join("third"))?;
    let original = temp.path().join("cache/original");
    let locations = ProductLocations::new(&first, &original)?;
    let expected = resolve_destination(&original)?;
    assert_eq!(locations.for_project(&first)?, expected);
    let second_cache = locations.for_project(&second)?;
    let third_cache = locations.for_project(&third)?;
    assert_ne!(second_cache, third_cache);
    assert_eq!(
        second_cache.parent(),
        Some(expected.parent().unwrap().join("projects").as_path())
    );
    for _ in 0..4 {
        assert_eq!(locations.for_project(&first)?, expected);
        assert_eq!(locations.for_project(&second)?, second_cache);
        assert_eq!(locations.for_project(&third)?, third_cache);
    }
    assert!(ProductLocations::new(&first, first.assets()).is_err());
    assert!(ProductLocations::new(&first, &first.assets().join("cache")).is_err());
    assert!(ProductLocations::new(&first, first.root()).is_err());
    let future = temp.path().join("not-created/project");
    let future_cache = locations.for_root(&future)?;
    assert!(!future.exists());
    let future_paths = empty_project(&future)?;
    assert_eq!(locations.for_project(&future_paths)?, future_cache);
    Ok(())
}

#[cfg(unix)]
#[test]
fn cache_identity_uses_canonical_projects_and_rechecks_symlink_overlap() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let first = empty_project(&temp.path().join("first"))?;
    let second = empty_project(&temp.path().join("second"))?;
    let original = temp.path().join("cache/original");
    let locations = ProductLocations::new(&first, &original)?;
    symlink(first.root(), temp.path().join("alias"))?;
    assert_eq!(
        locations.for_project(&ProjectPaths::open(temp.path().join("alias"))?)?,
        resolve_destination(&original)?
    );
    symlink(second.root(), temp.path().join("second-alias"))?;
    assert_eq!(
        locations.for_project(&ProjectPaths::open(temp.path().join("second-alias"))?)?,
        locations.for_project(&second)?
    );
    fs::create_dir_all(temp.path().join("cache"))?;
    symlink(second.assets(), temp.path().join("cache/projects"))?;
    assert!(locations.for_project(&second).is_err());
    Ok(())
}

#[test]
fn background_load_returns_a_complete_owned_candidate_and_prevents_overlapping_jobs() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    fixture(&root)?;
    let products = temp.path().join("products");
    let mut load = ProjectLoad::default();
    load.start(root.clone(), products.clone(), "main.scene".into(), None)?;
    assert!(load.is_running());
    assert!(
        load.start(root.clone(), products, "main.scene".into(), None)
            .is_err()
    );
    let mut candidate = wait(&mut load).map_err(anyhow::Error::msg)?;
    assert!(!load.is_running());
    assert!(load.failure.is_none());
    assert_eq!(candidate.paths.root(), root.canonicalize()?);
    assert_eq!(
        candidate.source,
        root.join("assets/main.scene").canonicalize()?
    );
    assert_eq!(
        candidate.source_asset,
        SourceMeta::load(&candidate.source)?.id
    );
    assert_eq!(candidate.prefab_asset(), None);
    assert!(
        !candidate
            .sources
            .of_kind(comet_asset_data::product::AssetKind::Scene)
            .collect::<Vec<_>>()
            .is_empty()
    );
    assert_eq!(
        &*candidate.snapshot,
        &candidate.content.expand(&mut candidate.assets)?
    );
    comet_application::extract(&candidate.snapshot, &mut candidate.assets, &[])?;
    let document = comet_editor_core::Document::open_source(candidate.content)?;
    assert!(!document.is_dirty());
    Ok(())
}

#[test]
fn native_prefabs_load_multiple_roots_and_nested_declarations_without_remapping_the_source()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    prefab_fixture(&root)?;
    let source = root.join("assets/branch.prefab");
    let mut content = ContentSource::from_json(&fs::read(&source)?)?;
    let independent = comet_scene::EntityData::new("独立作者根");
    let independent_id = independent.id;
    content.base.entities.push(independent);
    let source_bytes = content.to_json()?;
    fs::write(&source, &source_bytes)?;
    let content = ContentSource::from_json(&source_bytes)?;
    let metadata = fs::read(SourceMeta::path(&source))?;
    let identity = SourceMeta::load(&source)?.id;
    let products = temp.path().join("products");

    let mut load = ProjectLoad::default();
    load.start(root.clone(), products.clone(), "branch.prefab".into(), None)?;
    let mut candidate = wait(&mut load).map_err(anyhow::Error::msg)?;
    assert_eq!(candidate.source_asset, identity);
    assert_eq!(candidate.prefab_asset(), Some(identity));
    assert_eq!(candidate.source, source.canonicalize()?);
    assert_eq!(candidate.content, content);
    assert!(
        candidate
            .sources
            .of_kind(comet_asset_data::product::AssetKind::Scene)
            .next()
            .is_none()
    );
    assert!(
        candidate
            .snapshot
            .entities
            .iter()
            .any(|entity| { entity.id == independent_id && entity.parent.is_none() })
    );
    for root in &content.base.entities {
        assert!(
            candidate
                .snapshot
                .entities
                .iter()
                .any(|entity| entity == root)
        );
    }
    let instance = &content.instances[0];
    let product = candidate.assets.load(instance.prefab)?;
    let Product::Prefab(nested) = &product.payload else {
        unreachable!()
    };
    let nested_root = nested
        .base
        .entities
        .iter()
        .find(|entity| entity.parent.is_none())
        .unwrap();
    let expanded_nested = candidate
        .snapshot
        .entities
        .iter()
        .find(|entity| entity.id == nested_root.id.in_instance(instance.id))
        .unwrap();
    assert_eq!(expanded_nested.parent, instance.parent);
    assert_eq!(
        expanded_nested.model.as_ref().unwrap().surface.roughness,
        0.25
    );
    assert_eq!(
        &*candidate.snapshot,
        &content.expand_prefab(identity, &mut candidate.assets)?
    );
    comet_application::extract(&candidate.snapshot, &mut candidate.assets, &[])?;
    candidate.verify_current()?;
    let reopened = ProjectCandidate::load(root, products, "branch.prefab".into())?;
    assert_eq!(reopened.source_asset, identity);
    assert_eq!(reopened.content, content);
    assert_eq!(reopened.snapshot, candidate.snapshot);
    assert_eq!(fs::read(&source)?, source_bytes);
    assert_eq!(fs::read(SourceMeta::path(&source))?, metadata);
    Ok(())
}

#[test]
fn a_project_containing_only_one_native_prefab_needs_no_scene_asset() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    empty_project(&root)?;
    let source = root.join("assets/unit.prefab");
    let mut content = ContentSource::default();
    content
        .base
        .entities
        .push(comet_scene::EntityData::new("根一"));
    content
        .base
        .entities
        .push(comet_scene::EntityData::new("根二"));
    fs::write(&source, content.to_json()?)?;
    let candidate =
        ProjectCandidate::load(root, temp.path().join("products"), "unit.prefab".into())?;
    assert_eq!(candidate.prefab_asset(), Some(candidate.source_asset));
    assert_eq!(SourceMeta::load(&source)?.id, candidate.source_asset);
    assert_eq!(candidate.assets.index().products.len(), 1);
    assert!(matches!(
        candidate.assets.index().products[&candidate.source_asset].kind,
        comet_asset_data::product::AssetKind::Prefab
    ));
    assert_eq!(candidate.snapshot.entities.len(), 2);
    assert!(
        candidate
            .snapshot
            .entities
            .iter()
            .all(|entity| entity.parent.is_none())
    );
    candidate.verify_current()?;
    Ok(())
}

#[test]
fn unsorted_native_author_documents_load_without_rewriting_sources_metadata_or_products()
-> Result<()> {
    for filename in ["unsorted.scene", "unsorted.prefab"] {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("project");
        prefab_fixture(&root)?;
        let mut unsorted = ContentSource::from_json(&fs::read(root.join("assets/branch.prefab"))?)?;
        unsorted
            .base
            .entities
            .push(comet_scene::EntityData::new("第二作者根"));
        unsorted.base.entities.sort_by_key(|entity| entity.id);
        unsorted.base.entities.reverse();
        let mut other = unsorted.instances[0].clone();
        other.id = comet_core::InstanceId::new();
        other.parent = None;
        unsorted.instances.push(other);
        unsorted.instances.sort_by_key(|instance| instance.id);
        unsorted.instances.reverse();
        for instance in &mut unsorted.instances {
            instance.overrides.push(comet_scene::FieldOverride {
                entity: instance.overrides[0].entity,
                field: comet_scene::schema::TRANSLATION,
                value: comet_scene::schema::FieldValue::Vec3([0., 1., 0.]),
            });
            instance
                .overrides
                .sort_by_key(|value| (value.entity, value.field.component, value.field.field));
            instance.overrides.reverse();
        }
        let source = root.join("assets").join(filename);
        let bytes = serde_json::to_vec_pretty(&unsorted)?;
        let content = ContentSource::from_json(&bytes)?;
        assert_ne!(unsorted, content);
        let identity = AssetId::new();
        let metadata = serde_json::to_vec_pretty(&SourceMeta {
            version: 1,
            id: identity,
        })?;
        fs::write(&source, &bytes)?;
        fs::write(SourceMeta::path(&source), &metadata)?;

        let mut candidate =
            ProjectCandidate::load(root, temp.path().join("products"), filename.into())?;
        assert_eq!(candidate.source_asset, identity);
        assert_eq!(candidate.content, content);
        let product = candidate.assets.load(identity)?;
        let built = match &product.payload {
            Product::Scene(built) | Product::Prefab(built) => built,
            _ => unreachable!(),
        };
        assert_eq!(built, &unsorted);
        let snapshot = if filename.ends_with("prefab") {
            assert_eq!(candidate.prefab_asset(), Some(identity));
            content.expand_prefab(identity, &mut candidate.assets)?
        } else {
            assert_eq!(candidate.prefab_asset(), None);
            content.expand(&mut candidate.assets)?
        };
        assert_eq!(&*candidate.snapshot, &snapshot);
        assert_eq!(fs::read(&source)?, bytes);
        assert_eq!(fs::read(SourceMeta::path(&source))?, metadata);
        candidate.verify_current()?;
    }
    Ok(())
}

#[test]
fn an_identical_product_of_the_other_author_kind_cannot_replace_the_selected_asset() -> Result<()> {
    for filename in ["unit.scene", "unit.prefab"] {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("project");
        empty_project(&root)?;
        let content = ContentSource::default().to_json()?;
        let scene = root.join("assets/unit.scene");
        let prefab = root.join("assets/unit.prefab");
        fs::write(&scene, &content)?;
        fs::write(&prefab, &content)?;
        let selected = root.join("assets").join(filename);
        let other = if filename.ends_with("prefab") {
            &scene
        } else {
            &prefab
        };
        let mut replaced = false;
        let candidate = ProjectCandidate::prepare(
            root,
            temp.path().join("products"),
            filename.into(),
            None,
            |stage| {
                if stage == "正在校验作者文件及依赖产品" {
                    let metadata = fs::read(SourceMeta::path(other)).unwrap();
                    fs::write(SourceMeta::path(&selected), metadata).unwrap();
                    replaced = true;
                }
            },
        );
        assert!(replaced);
        assert!(candidate.is_err_and(|error| error.to_string().contains("产品类型不匹配")));
    }
    Ok(())
}

#[test]
fn failed_load_preserves_an_accepted_store_and_retry_reuses_the_same_project_cache() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    fixture(&root)?;
    let products = temp.path().join("products");
    let mut accepted = ProjectCandidate::load(root.clone(), products.clone(), "main.scene".into())?;
    let revision = accepted.assets.revision().to_owned();
    let before = accepted.snapshot.clone();
    let buffer = root.join("assets/showcase.bin");
    let bytes = fs::read(&buffer)?;
    fs::write(&buffer, b"broken buffer")?;
    let mut load = ProjectLoad::default();
    load.start(root.clone(), products.clone(), "main.scene".into(), None)?;
    assert!(wait(&mut load).is_err());
    assert!(!load.is_running());
    assert!(load.failure.is_some());
    assert_eq!(accepted.assets.revision(), revision);
    assert_eq!(accepted.snapshot, before);
    comet_application::extract(&accepted.snapshot, &mut accepted.assets, &[])?;
    fs::write(buffer, bytes)?;
    load.start(root, products, "main.scene".into(), None)?;
    let recovered = wait(&mut load).map_err(anyhow::Error::msg)?;
    assert!(load.failure.is_none());
    assert_eq!(recovered.snapshot, accepted.snapshot);
    assert_eq!(recovered.products, accepted.products);
    Ok(())
}

#[test]
fn project_open_preserves_settings_across_failed_imports_and_retries() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    fixture(&root)?;
    let configuration = root.join("project.json");
    let saved = fs::read(&configuration)?;
    let buffer = root.join("assets/showcase.bin");
    let bytes = fs::read(&buffer)?;
    fs::write(&buffer, b"invalid source buffer")?;
    let products = temp.path().join("products");
    assert!(ProjectCandidate::load(root.clone(), products.clone(), "main.scene".into()).is_err());
    let paths = ProjectPaths::open(&root)?;
    let initialized = comet_asset_pipeline::ProjectSettings::load(&paths)?;
    assert_eq!(initialized.version, 2);
    assert_eq!(fs::read(&configuration)?, saved);
    fs::write(&buffer, bytes)?;
    let accepted = ProjectCandidate::load(root.clone(), products, "main.scene".into())?;
    assert_eq!(accepted.panel.game_id(), initialized.game_id);
    accepted.verify_current()?;
    assert_eq!(fs::read(&configuration)?, saved);
    Ok(())
}

#[test]
fn project_identity_changes_during_resource_preparation_cannot_be_accepted() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    fixture(&root)?;
    let configuration = root.join("project.json");
    let mut changed = false;
    let result = ProjectCandidate::prepare(
        root,
        temp.path().join("products"),
        "main.scene".into(),
        None,
        |stage| {
            if stage == "正在构建项目资源" {
                fs::write(
                    &configuration,
                    serde_json::to_vec(&comet_asset_pipeline::ProjectSettings::for_new_project())
                        .unwrap(),
                )
                .unwrap();
                changed = true;
            }
        },
    );
    assert!(changed);
    assert!(result.is_err_and(|error| error.to_string().contains("项目设置")));
    Ok(())
}

#[test]
fn invalid_scene_paths_and_cache_overlap_fail_before_creating_products() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("project");
    fixture(&root)?;
    let outside = root.join("outside.scene");
    fs::copy(root.join("assets/main.scene"), &outside)?;
    fs::write(root.join("assets/showcase.glb"), b"not an author document")?;
    let products = temp.path().join("products");
    for source in [
        outside,
        "../outside.scene".into(),
        "showcase.gltf".into(),
        "showcase.glb".into(),
    ] {
        assert!(ProjectCandidate::load(root.clone(), products.clone(), source).is_err());
        assert!(!products.exists());
    }
    let invalid_cache = root.join("assets/products");
    assert!(ProjectCandidate::load(root, invalid_cache.clone(), "main.scene".into()).is_err());
    assert!(!invalid_cache.exists());
    Ok(())
}

#[test]
fn background_creation_marks_publication_and_can_retry_loading_without_changing_identity()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("new-project");
    let products = temp.path().join("blocked-cache");
    fs::write(&products, b"existing file")?;
    let mut load = ProjectLoad::default();
    load.create(root.clone(), products.clone(), None)?;
    assert!(wait(&mut load).is_err());
    assert!(load.created());
    assert!(load.failure.is_some());
    let source = root
        .join("assets")
        .join(comet_asset_pipeline::DEFAULT_SCENE);
    let identity = SourceMeta::load(&source)?.id;
    let metadata = fs::read(SourceMeta::path(&source))?;
    assert_eq!(fs::read(&products)?, b"existing file");
    fs::remove_file(&products)?;
    load.start(
        root.clone(),
        products.clone(),
        comet_asset_pipeline::DEFAULT_SCENE.into(),
        None,
    )?;
    assert!(!load.created());
    let recovered = wait(&mut load).map_err(anyhow::Error::msg)?;
    assert_eq!(recovered.source_asset, identity);
    assert_eq!(fs::read(SourceMeta::path(&source))?, metadata);
    assert!(load.failure.is_none());
    // Existing project locations are never replaced by another create attempt.
    load.create(root, products, None)?;
    assert!(wait(&mut load).is_err());
    assert!(!load.created());
    assert_eq!(fs::read(SourceMeta::path(&source))?, metadata);
    Ok(())
}

#[test]
fn creating_a_project_with_an_asset_cache_overlap_does_not_publish_a_directory() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("new-project");
    let products = root.join("assets/products");
    let mut load = ProjectLoad::default();
    load.create(root.clone(), products, None)?;
    assert!(wait(&mut load).is_err());
    assert!(!load.created());
    assert!(!root.exists());
    Ok(())
}

#[test]
fn source_changes_after_build_cannot_be_accepted_with_older_products() -> Result<()> {
    for filename in ["main.scene", "branch.prefab"] {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("project");
        if filename.ends_with("prefab") {
            prefab_fixture(&root)?;
        } else {
            fixture(&root)?;
        }
        let source = root.join("assets").join(filename);
        let mut changed = ContentSource::from_json(&fs::read(&source)?)?;
        changed
            .base
            .entities
            .push(comet_scene::EntityData::new("late authoring change"));
        let bytes = changed.to_json()?;
        let mut wrote = false;
        let result = ProjectCandidate::prepare(
            root,
            temp.path().join("products"),
            filename.into(),
            None,
            |stage| {
                if stage == "正在校验作者文件及依赖产品" {
                    fs::write(&source, &bytes).unwrap();
                    wrote = true;
                }
            },
        );
        assert!(wrote);
        assert!(result.is_err_and(|error| error.to_string().contains("构建后发生变化")));
    }
    Ok(())
}

#[test]
fn completed_candidates_reject_later_source_metadata_and_settings_changes() -> Result<()> {
    for (filename, change) in ["main.scene", "branch.prefab"]
        .into_iter()
        .flat_map(|filename| (0..3).map(move |change| (filename, change)))
    {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("project");
        if filename.ends_with("prefab") {
            prefab_fixture(&root)?;
        } else {
            fixture(&root)?;
        }
        let candidate =
            ProjectCandidate::load(root.clone(), temp.path().join("products"), filename.into())?;
        candidate.verify_current()?;
        match change {
            0 => {
                let mut content = candidate.content.clone();
                content
                    .base
                    .entities
                    .push(comet_scene::EntityData::new("external edit"));
                fs::write(&candidate.source, content.to_json()?)?;
            }
            1 => fs::write(
                SourceMeta::path(&candidate.source),
                serde_json::to_vec(&SourceMeta {
                    version: 1,
                    id: AssetId::new(),
                })?,
            )?,
            _ => fs::write(
                root.join("project.json"),
                serde_json::to_vec(&comet_asset_pipeline::ProjectSettings {
                    game_icon: Some(AssetId::new()),
                    ..candidate.panel.settings.clone()
                })?,
            )?,
        }
        let error = candidate.verify_current().unwrap_err().to_string();
        assert!(
            error.contains(["作者文件在加载", "作者文件身份", "项目设置"][change]),
            "{error}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn completed_candidates_recheck_the_actual_scene_and_product_paths() -> Result<()> {
    use std::os::unix::fs::symlink;
    for change_source in [false, true] {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("project");
        fixture(&root)?;
        let products = temp.path().join("products");
        let candidate =
            ProjectCandidate::load(root.clone(), products.clone(), "main.scene".into())?;
        if change_source {
            let replacement = root.join("assets/renamed.scene");
            fs::rename(&candidate.source, &replacement)?;
            symlink(&replacement, &candidate.source)?;
        } else {
            let replacement = temp.path().join("moved-products");
            fs::rename(&products, &replacement)?;
            symlink(&replacement, &products)?;
        }
        assert!(candidate.verify_current().is_err());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn background_open_and_create_never_write_products_into_the_protected_project_assets() -> Result<()>
{
    use std::os::unix::fs::symlink;
    for create in [false, true] {
        let temp = tempfile::tempdir()?;
        let initial = empty_project(&temp.path().join("first"))?;
        let current = empty_project(&temp.path().join("second"))?;
        let target = temp.path().join("third");
        if !create {
            fixture(&target)?;
        }
        let locations = ProductLocations::new(&initial, &temp.path().join("cache/initial"))?;
        let products = locations.for_root(&target)?;
        fs::create_dir_all(products.parent().unwrap())?;
        symlink(current.assets(), &products)?;
        // The target and initial project's assets do not cover the currently open project.
        assert_eq!(locations.for_root(&target)?, current.assets());
        let mut load = ProjectLoad::default();
        let protected = Some(current.assets().to_owned());
        if create {
            load.create(target.clone(), products, protected)?;
        } else {
            load.start(target.clone(), products, "main.scene".into(), protected)?;
        }
        assert!(wait(&mut load).is_err());
        assert!(fs::read_dir(current.assets())?.next().is_none());
        assert!(!load.created());
        if create {
            assert!(!target.exists());
        }
    }
    Ok(())
}
