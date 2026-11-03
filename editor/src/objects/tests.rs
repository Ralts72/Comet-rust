use crate::test_support::fixture;
use anyhow::Result;
use comet_asset::AssetStore;
use std::fs;

#[test]
fn scene_object_commands_support_undo_and_are_blocked_during_play() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let before = editor.document.source();
    editor.create_object(None, &mut assets)?;
    let root = editor.selected.unwrap();
    editor.create_object(Some(root), &mut assets)?;
    let child = editor.selected.unwrap();
    editor.duplicate_object(root, &mut assets)?;
    let copy = editor.selected.unwrap();
    assert_ne!(copy, root);
    assert!(
        editor
            .document
            .source()
            .base
            .entities
            .iter()
            .any(|e| e.parent == Some(copy))
    );
    editor.delete_object(root, &mut assets)?;
    assert!(
        !editor
            .document
            .source()
            .base
            .entities
            .iter()
            .any(|e| e.id == root || e.id == child)
    );
    editor.document.undo()?;
    assert!(
        editor
            .document
            .source()
            .base
            .entities
            .iter()
            .any(|e| e.id == child)
    );
    editor.document.undo()?;
    editor.document.undo()?;
    editor.document.undo()?;
    assert_eq!(editor.document.source(), before);
    editor.start(&mut assets)?;
    assert!(editor.create_object(None, &mut assets).is_err());
    assert!(editor.duplicate_object(root, &mut assets).is_err());
    assert!(editor.delete_object(root, &mut assets).is_err());
    assert_eq!(editor.document.source(), before);
    Ok(())
}

#[test]
fn placing_a_resource_preserves_its_instance_and_undoes_as_one_edit() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.document = comet_editor_core::Document::open(comet_scene::SceneSnapshot::default())?;
    let choice = editor
        .sources
        .of_kind(comet_asset_data::product::AssetKind::Prefab)
        .find(|choice| choice.source == std::path::Path::new("showcase.gltf"))
        .unwrap()
        .clone();
    let before = editor.document.source();
    let count = editor.snapshot(&mut assets)?.entities.len();
    editor.place_resource(choice.id, "showcase", &mut assets)?;
    let container = editor.selected.unwrap();
    let placed = editor.document.source();
    assert_eq!(placed.instances.len(), before.instances.len() + 1);
    assert!(
        placed
            .instances
            .iter()
            .any(|instance| instance.prefab == choice.id && instance.parent == Some(container))
    );
    assert!(editor.snapshot(&mut assets)?.entities.len() > count + 1);
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.source(), placed);
    editor.save(&mut assets)?;
    assert_eq!(
        comet_editor_core::Document::load(&editor.source)?.source(),
        placed
    );
    comet_asset_pipeline::prepare(
        editor.source.parent().unwrap().parent().unwrap(),
        &editor.products,
    )?;
    let mut rebuilt = AssetStore::open(&editor.products)?;
    let meta: comet_asset_pipeline::SourceMeta =
        serde_json::from_slice(&fs::read(format!("{}.meta", editor.source.display()))?)?;
    assert!(rebuilt.expand_scene(meta.id)?.entities.len() > 1);
    let history = editor.document.history_len();
    assert!(
        editor
            .place_resource(comet_core::AssetId::new(), "missing", &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.source(), placed);
    assert_eq!(editor.document.history_len(), history);
    editor.start(&mut assets)?;
    assert!(
        editor
            .place_resource(choice.id, "showcase", &mut assets)
            .is_err()
    );
    Ok(())
}
