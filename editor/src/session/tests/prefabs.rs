use super::*;
use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::SourceMeta;
use comet_core::{AssetId, InstanceId};
use comet_scene::{PrefabInstance, PrefabResolver};
use std::{fs, sync::Arc};

fn write_source(path: &std::path::Path, id: AssetId, source: &ContentSource) -> Result<()> {
    fs::write(path, source.to_json()?)?;
    fs::write(
        SourceMeta::path(path),
        serde_json::to_vec(&SourceMeta { version: 1, id })?,
    )?;
    Ok(())
}

fn prefab(editor: &Editor) -> Result<(AssetId, EntityId)> {
    let source_id = AssetId::new();
    let mut root = EntityData::new("editable source");
    root.model = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .find_map(|entity| entity.model.clone());
    let root_id = root.id;
    let mut child = EntityData::new("child");
    child.parent = Some(root_id);
    let imported = editor.sources.of_kind(AssetKind::Prefab).next().unwrap().id;
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![root, child, EntityData::new("second root")],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            id: InstanceId::new(),
            prefab: imported,
            parent: Some(root_id),
            overrides: vec![],
        }],
        ..Default::default()
    };
    write_source(
        &editor.paths.assets().join("editable.prefab"),
        source_id,
        &source,
    )?;
    Ok((source_id, root_id))
}

#[test]
fn source_edits_keep_identities_and_reach_instances_only_after_build_acceptance() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let (source_id, root) = prefab(&editor)?;
    let scene_id = SourceMeta::load(&editor.source)?.id;
    let instance = InstanceId::new();
    let scene = ContentSource {
        instances: vec![PrefabInstance {
            id: instance,
            prefab: source_id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    fs::write(&editor.source, scene.to_json()?)?;
    editor.switch_document("editable.prefab".into(), &mut assets)?;
    let metadata = fs::read(SourceMeta::path(&editor.source))?;
    let original = editor.document.shared_source();
    let nested = original.instances.clone();
    assert_eq!(editor.document.prefab_asset(), Some(source_id));
    assert_eq!(editor.export.entry_scene(), Some(scene_id));

    editor.edit(
        root,
        schema::TRANSLATION,
        FieldValue::Vec3([2., 0., 0.]),
        &mut assets,
    )?;
    editor.commit()?;
    let edited = editor.document.shared_source();
    editor.save(&mut assets)?;
    assert_eq!(fs::read(SourceMeta::path(&editor.source))?, metadata);
    assert_eq!(editor.document.shared_source().instances, nested);
    assert_eq!(
        assets
            .prefab(source_id)?
            .base
            .entities
            .iter()
            .find(|entity| entity.id == root)
            .unwrap()
            .transform
            .translation,
        [0.; 3]
    );
    editor.refresh_resources()?;
    super::resources::wait(&mut editor, &mut assets)?;
    assert!(
        editor.resource_build.failure.is_none(),
        "{:?}",
        editor.resource_build.failure
    );
    let expanded = assets.expand_scene(scene_id)?;
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|entity| entity.id == root.in_instance(instance))
            .unwrap()
            .transform
            .translation,
        [2., 0., 0.]
    );
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), original);
    assert!(editor.document.is_dirty());
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), edited);
    assert!(!editor.document.is_dirty());

    editor.switch_document("main.scene".into(), &mut assets)?;
    assert_eq!(editor.document.prefab_asset(), None);
    assert_eq!(editor.export.entry_scene(), Some(scene_id));
    assert_eq!(editor.snapshot(&mut assets)?, expanded);
    editor.switch_document("editable.prefab".into(), &mut assets)?;
    assert_eq!(editor.document.shared_source(), edited);
    assert_eq!(editor.document.history_len(), 0);
    assert_eq!(fs::read(SourceMeta::path(&editor.source))?, metadata);
    Ok(())
}

#[test]
fn prefab_preview_and_invalid_export_never_replace_or_save_the_author_document() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let (source_id, root) = prefab(&editor)?;
    editor.switch_document("editable.prefab".into(), &mut assets)?;
    editor.rename_object(root, "unsaved source".into())?;
    editor.commit()?;
    let source = editor.document.shared_source();
    let history = editor.document.history_len();
    let saved = fs::read(&editor.source)?;
    editor.start(&mut assets)?;
    let first_world = editor.world();
    editor.edit(
        root,
        schema::TRANSLATION,
        FieldValue::Vec3([3., 0., 0.]),
        &mut assets,
    )?;
    editor.restart_play()?;
    assert_ne!(editor.world(), first_world);
    assert_eq!(
        editor
            .play
            .as_mut()
            .unwrap()
            .snapshot()
            .entities
            .iter()
            .find(|entity| entity.id == root)
            .unwrap()
            .transform
            .translation,
        [0.; 3]
    );
    let world = editor.world();
    for (entry, additional) in [
        (source_id, vec![]),
        (editor.export.entry_scene().unwrap(), vec![source_id]),
    ] {
        let error = editor
            .begin_export(
                &mut assets,
                crate::export::ExportAction {
                    entry,
                    additional,
                    output: editor.products.parent().unwrap().join("invalid-export"),
                    run: false,
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("必须是场景"));
        assert_eq!(editor.world(), world);
        assert_eq!(fs::read(&editor.source)?, saved);
    }
    editor.stop();
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), history);
    assert!(editor.document.is_dirty());
    assert_eq!(fs::read(&editor.source)?, saved);
    Ok(())
}

#[test]
fn prefab_root_cycles_are_rejected_for_current_edits_and_retained_history_refreshes() -> Result<()>
{
    let (_temp, mut editor, mut assets) = fixture()?;
    let (source_id, _) = prefab(&editor)?;
    let other = AssetId::new();
    let other_path = editor.paths.assets().join("other.prefab");
    write_source(&other_path, other, &ContentSource::default())?;
    editor.switch_document("editable.prefab".into(), &mut assets)?;
    let original = editor.document.shared_source();
    assert!(
        editor
            .place_resource(source_id, "self", &mut assets)
            .is_err()
    );
    assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), 0);

    editor.place_resource(other, "history only", &mut assets)?;
    let retained = editor.document.shared_source();
    editor.document.undo()?;
    let accepted = assets.shared_index();
    let source = editor.document.shared_source();
    let back_reference = ContentSource {
        instances: vec![PrefabInstance {
            id: InstanceId::new(),
            prefab: source_id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    fs::write(&other_path, back_reference.to_json()?)?;
    editor.refresh_resources()?;
    super::resources::wait(&mut editor, &mut assets)?;
    assert!(
        editor
            .resource_build
            .failure
            .as_ref()
            .is_some_and(|error| error.contains("cycle")),
        "{:?}",
        editor.resource_build.failure
    );
    assert!(Arc::ptr_eq(&accepted, &assets.shared_index()));
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), retained);
    editor.document.undo()?;

    fs::write(&other_path, ContentSource::default().to_json()?)?;
    editor.refresh_resources()?;
    super::resources::wait(&mut editor, &mut assets)?;
    assert!(editor.resource_build.failure.is_none());
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), retained);
    Ok(())
}

#[test]
fn a_changed_prefab_metadata_identity_cannot_save_or_accept_a_resource_candidate() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let (_, root) = prefab(&editor)?;
    editor.switch_document("editable.prefab".into(), &mut assets)?;
    let metadata_path = SourceMeta::path(&editor.source);
    let metadata = fs::read(&metadata_path)?;
    let saved = fs::read(&editor.source)?;
    editor.rename_object(root, "live preview".into())?;
    let preview = editor.document.shared_source();
    let replacement = serde_json::to_vec(&SourceMeta {
        version: 1,
        id: AssetId::new(),
    })?;
    fs::write(&metadata_path, &replacement)?;
    assert!(editor.save(&mut assets).is_err());
    assert!(editor.refresh_resources().is_err());
    assert!(editor.document.is_editing());
    assert!(Arc::ptr_eq(&preview, &editor.document.shared_source()));
    assert_eq!(fs::read(&editor.source)?, saved);
    assert_eq!(fs::read(&metadata_path)?, replacement);

    fs::write(&metadata_path, &metadata)?;
    editor.save(&mut assets)?;
    let accepted = assets.shared_index();
    editor.refresh_resources()?;
    fs::write(&metadata_path, replacement)?;
    super::resources::wait(&mut editor, &mut assets)?;
    assert!(editor.resource_build.failure.is_some());
    assert!(Arc::ptr_eq(&accepted, &assets.shared_index()));
    assert!(Arc::ptr_eq(&preview, &editor.document.shared_source()));
    assert!(!editor.document.is_dirty());
    assert_eq!(editor.document.history_len(), 1);
    Ok(())
}
