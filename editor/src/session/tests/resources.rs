use super::*;
use comet_asset_data::product::AssetKind;
use std::{
    fs,
    time::{Duration, Instant},
};

fn wait(editor: &mut Editor, assets: &mut AssetStore) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.resource_build.is_running() {
        editor.poll_resources(assets);
        anyhow::ensure!(Instant::now() < deadline, "resource worker timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn new_prefab(editor: &Editor) -> Result<PathBuf> {
    let file = editor.paths.assets().join("新增.prefab");
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![EntityData::new("新增对象")],
            ..Default::default()
        },
        ..Default::default()
    };
    fs::write(&file, source.to_json()?)?;
    Ok(file)
}

#[test]
fn refresh_rebuilds_models_and_discovers_sources_without_saving_authoring() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let snapshot = editor.snapshot(&mut assets)?;
    let mesh = snapshot
        .entities
        .iter()
        .find_map(|entity| entity.model.as_ref().map(|model| model.mesh))
        .unwrap();
    let previous_product = assets.load(mesh)?;
    let revision = assets.revision().to_owned();
    let saved = fs::read(&editor.source)?;
    let id = snapshot.entities[0].id;
    editor.edit(
        id,
        schema::TRANSLATION,
        schema::FieldValue::Vec3([2., 0., 0.]),
        &mut assets,
    )?;
    let authored = editor.document.shared_source();
    let file = new_prefab(&editor)?;
    let buffer = editor.paths.assets().join("showcase.bin");
    let mut bytes = fs::read(&buffer)?;
    bytes[..4].copy_from_slice(&(-0.6_f32).to_le_bytes());
    fs::write(buffer, bytes)?;
    editor.refresh_resources()?;
    assert!(editor.refresh_resources().is_err());
    assert!(editor.start(&mut assets).is_err());
    assert!(editor.save(&mut assets).is_err());
    wait(&mut editor, &mut assets)?;
    assert!(
        editor.resource_build.failure.is_none(),
        "{:?}",
        editor.resource_build.failure
    );
    assert_ne!(assets.revision(), revision);
    assert_ne!(
        assets.load(mesh)?.source_digest,
        previous_product.source_digest
    );
    let added = SourceMeta::load(&file)?.id;
    assert!(
        editor
            .sources
            .of_kind(AssetKind::Prefab)
            .any(|source| source.id == added)
    );
    assert_eq!(editor.document.shared_source(), authored);
    assert!(editor.document.is_dirty());
    assert_eq!(fs::read(&editor.source)?, saved);
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert!(!editor.document.is_dirty());
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), authored);
    editor.start(&mut assets)?;
    assert!(editor.refresh_resources().is_err());
    Ok(())
}

#[test]
fn failed_refresh_keeps_store_catalog_document_and_snapshot_and_can_retry() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let accepted = editor.shared_snapshot(&mut assets)?;
    let revision = assets.revision().to_owned();
    let document = editor.document.shared_source();
    let choices = editor.sources.of_kind(AssetKind::Prefab).count();
    let file = editor.paths.assets().join("bad.gltf");
    fs::write(&file, b"invalid gltf")?;
    editor.refresh_resources()?;
    wait(&mut editor, &mut assets)?;
    assert!(editor.resource_build.failure.is_some());
    assert_eq!(assets.revision(), revision);
    assert_eq!(editor.sources.of_kind(AssetKind::Prefab).count(), choices);
    assert_eq!(editor.document.shared_source(), document);
    assert!(std::sync::Arc::ptr_eq(
        &accepted,
        &editor.shared_snapshot(&mut assets)?
    ));
    fs::remove_file(file)?;
    editor.refresh_resources()?;
    wait(&mut editor, &mut assets)?;
    assert!(editor.resource_build.failure.is_none());
    Ok(())
}

#[test]
fn refresh_rejects_products_needed_only_by_undo_or_redo() -> Result<()> {
    for redo in [false, true] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let file = new_prefab(&editor)?;
        editor.refresh_resources()?;
        wait(&mut editor, &mut assets)?;
        let prefab = SourceMeta::load(&file)?.id;
        editor.place_resource(prefab, "history", &mut assets)?;
        let placed = editor.document.shared_source();
        if redo {
            editor.document.undo()?;
        } else {
            editor.delete_object(editor.selected.unwrap(), &mut assets)?;
        }
        let current = editor.document.shared_source();
        let revision = assets.revision().to_owned();
        fs::remove_file(&file)?;
        fs::remove_file(SourceMeta::path(&file))?;
        editor.refresh_resources()?;
        wait(&mut editor, &mut assets)?;
        assert!(
            editor
                .resource_build
                .failure
                .as_ref()
                .is_some_and(|error| error.contains("历史"))
        );
        assert_eq!(assets.revision(), revision);
        assert_eq!(editor.document.shared_source(), current);
        if redo {
            editor.document.redo()?;
        } else {
            editor.document.undo()?;
        }
        assert_eq!(editor.document.shared_source(), placed);
        editor.snapshot(&mut assets)?;
    }
    Ok(())
}

#[test]
fn closing_waits_for_refresh_and_stale_document_candidates_are_rejected() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let revision = assets.revision().to_owned();
    editor.refresh_resources()?;
    assert!(
        editor
            .switch_scene(editor.source.clone(), &mut assets)
            .is_err()
    );
    assert!(!editor.request_editor_close());
    editor
        .document
        .insert_entity(EntityData::new("late edit"), &mut assets)?;
    let authored = editor.document.shared_source();
    wait(&mut editor, &mut assets)?;
    assert!(
        editor
            .resource_build
            .failure
            .as_ref()
            .is_some_and(|error| error.contains("文档"))
    );
    assert_eq!(assets.revision(), revision);
    assert_eq!(editor.document.shared_source(), authored);
    assert!(!editor.documents.approved);
    Ok(())
}
