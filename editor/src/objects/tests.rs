use crate::test_support::fixture;
use anyhow::Result;
use comet_asset::AssetStore;
use std::fs;

#[test]
fn scene_object_commands_support_undo_and_are_blocked_during_play() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let before = editor.document.source();
    editor.create_object(None, &mut assets)?;
    let root = editor.selection.active().unwrap();
    editor.create_object(Some(root), &mut assets)?;
    let child = editor.selection.active().unwrap();
    editor.duplicate_object(root, &mut assets)?;
    let copy = editor.selection.active().unwrap();
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
    let container = editor.selection.active().unwrap();
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

#[test]
fn reparent_commands_preserve_selection_and_reject_prefab_structure_and_play() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.create_object(None, &mut assets)?;
    let root = editor.selection.active().unwrap();
    editor.create_object(None, &mut assets)?;
    let child = editor.selection.active().unwrap();
    editor.reparent_object(child, Some(root), &mut assets)?;
    assert_eq!(editor.selection.active(), Some(child));
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
    assert!(
        editor
            .reparent_object(root, Some(child), &mut assets)
            .is_err()
    );
    let prefab_entity = editor
        .snapshot(&mut assets)?
        .entities
        .into_iter()
        .find(|entity| !before.base.entities.iter().any(|base| base.id == entity.id))
        .unwrap()
        .id;
    assert!(
        editor
            .reparent_object(prefab_entity, Some(root), &mut assets)
            .is_err()
    );
    assert!(
        editor
            .reparent_object(child, Some(prefab_entity), &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), history);
    editor.start(&mut assets)?;
    assert!(editor.reparent_object(child, None, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn rename_previews_keep_identity_and_merge_until_commit_or_cancel() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.create_object(None, &mut assets)?;
    let id = editor.selection.active().unwrap();
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
    for name in ["", "中文", "中文对象"] {
        editor.rename_object(id, name.into())?;
        assert_eq!(editor.document.history_len(), history);
    }
    editor.commit()?;
    assert_eq!(editor.document.history_len(), history + 1);
    let renamed = editor.document.shared_source();
    assert_eq!(
        renamed
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .name,
        "中文对象"
    );
    editor.save(&mut assets)?;
    assert_eq!(
        comet_editor_core::Document::load(&editor.source)?.shared_source(),
        renamed
    );
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), renamed);
    editor.rename_object(id, "临时名称".into())?;
    editor.cancel()?;
    assert_eq!(editor.document.shared_source(), renamed);
    editor.start(&mut assets)?;
    assert!(editor.rename_object(id, "运行中".into()).is_err());
    assert_eq!(editor.document.shared_source(), renamed);
    Ok(())
}

#[test]
fn mixed_invalid_group_is_rejected_without_history_or_partial_deletion() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.create_object(None, &mut assets)?;
    let authored = editor.selection.active().unwrap();
    let source = editor.document.shared_source();
    let snapshot = editor.snapshot(&mut assets)?;
    let interior = snapshot
        .entities
        .iter()
        .find(|entity| {
            entity.parent.is_some() && !source.base.entities.iter().any(|base| base.id == entity.id)
        })
        .unwrap()
        .id;
    editor.selection.replace([authored, interior]);
    let history = editor.document.history_len();
    assert!(editor.delete_object(authored, &mut assets).is_err());
    assert!(editor.duplicate_object(authored, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), source);
    assert_eq!(editor.document.history_len(), history);
    assert_eq!(editor.selection.len(), 2);
    Ok(())
}

fn physical_source(
    editor: &crate::session::Editor,
    assets: &mut AssetStore,
    entities: Vec<comet_scene::EntityData>,
) -> Result<comet_core::AssetId> {
    let paths = comet_core::ProjectPaths::open(editor.project_root())?;
    let path =
        comet_asset_pipeline::create_prefab(&paths, std::path::Path::new("commands.prefab"))?;
    let source = comet_scene::ContentSource {
        base: comet_scene::SceneSnapshot {
            version: comet_scene::scene::SCENE_VERSION,
            entities,
        },
        ..Default::default()
    };
    fs::write(&path, source.to_json()?)?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    Ok(comet_asset_pipeline::SourceMeta::load(&path)?.id)
}

#[test]
fn mixed_author_and_same_instance_roots_duplicate_once_and_keep_either_clicked_anchor() -> Result<()>
{
    use comet_scene::{EntityData, PhysicsBody, PrefabResolver};
    for clicked in 0..2 {
        let (_temp, mut editor, mut assets) = fixture()?;
        let entities = (0..2)
            .map(|index| {
                let mut entity = EntityData::new(format!("body {index}"));
                entity.physics = Some(PhysicsBody::default());
                entity.transform.translation = [index as f32 * 2., 2., 0.];
                entity
            })
            .collect();
        let prefab = physical_source(&editor, &mut assets, entities)?;
        let roots = assets.prefab(prefab)?.prefab_roots(&mut assets)?;
        let base_count = editor.document.shared_source().base.entities.len();
        editor.place_resource(prefab, "physical roots", &mut assets)?;
        assert_eq!(
            editor.document.shared_source().base.entities.len(),
            base_count
        );
        let instance = editor
            .document
            .shared_source()
            .instances
            .iter()
            .find(|instance| instance.prefab == prefab)
            .unwrap()
            .id;
        let old_roots: Vec<_> = roots
            .iter()
            .map(|root| root.in_instance(instance))
            .collect();
        editor.create_object(None, &mut assets)?;
        let author = editor.selection.active().unwrap();
        editor
            .selection
            .replace([old_roots[1 - clicked], author, old_roots[clicked]]);
        let before = editor.document.shared_source();
        let history = editor.document.history_len();
        editor.duplicate_object(old_roots[clicked], &mut assets)?;
        let after = editor.document.shared_source();
        assert_eq!(after.instances.len(), before.instances.len() + 1);
        assert_eq!(after.base.entities.len(), before.base.entities.len() + 1);
        assert_eq!(editor.document.history_len(), history + 1);
        let duplicate = after
            .instances
            .iter()
            .find(|entry| entry.prefab == prefab && entry.id != instance)
            .unwrap()
            .id;
        assert_eq!(editor.selection.len(), 3);
        for root in &roots {
            assert!(editor.selection.contains(root.in_instance(duplicate)));
        }
        assert_eq!(
            editor.selection.active(),
            Some(roots[clicked].in_instance(duplicate))
        );
        assert!(
            editor
                .snapshot(&mut assets)?
                .entities
                .iter()
                .filter(|entity| entity.physics.is_some())
                .all(|entity| entity.parent.is_none())
        );
        editor.delete_object(editor.selection.active().unwrap(), &mut assets)?;
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(editor.document.history_len(), history + 2);
        editor.document.undo()?;
        assert_eq!(editor.document.shared_source(), after);
        editor.document.redo()?;
        assert_eq!(editor.document.shared_source(), before);
    }
    Ok(())
}

#[test]
fn a_conflicting_player_prefab_cannot_publish_an_instance_or_change_selection_and_redo()
-> Result<()> {
    use comet_scene::{BodyMotion, Character, ColliderShape, EntityData, PhysicsBody};
    use std::sync::Arc;
    let (_temp, mut editor, mut assets) = fixture()?;
    let body = PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    };
    let mut player = EntityData::new("asset player");
    player.physics = Some(body.clone());
    player.character = Some(Character::default());
    let prefab = physical_source(&editor, &mut assets, vec![player])?;
    editor.create_object(None, &mut assets)?;
    let existing = editor.selection.active().unwrap();
    editor.edit_physics(
        existing,
        Some(body),
        Some(Character::default()),
        &mut assets,
    )?;
    editor.commit()?;
    editor.create_object(None, &mut assets)?;
    let redo = editor.document.shared_source();
    editor.document.undo()?;
    editor.selection.replace([existing]);
    let source = editor.document.shared_source();
    let state = editor.document.state_id();
    let revision = editor.document.revision();
    let history = editor.document.history_len();
    assert!(
        editor
            .place_resource(prefab, "conflicting player", &mut assets)
            .is_err()
    );
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    assert_eq!(editor.document.state_id(), state);
    assert_eq!(editor.document.revision(), revision);
    assert_eq!(editor.document.history_len(), history);
    assert_eq!(editor.selection.ids(), vec![existing]);
    assert_eq!(editor.selection.active(), Some(existing));
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), redo);
    Ok(())
}
