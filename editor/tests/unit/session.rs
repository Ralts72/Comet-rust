use super::*;
use crate::test_support::fixture;
use comet_render::{PickFeedback, ViewToken};
use comet_scene::schema;
#[test]
fn authored_physics_survives_save_reopen_and_cannot_change_during_play() -> Result<()> {
    use comet_scene::{BodyMotion, ColliderShape};
    let (temp, mut editor, mut assets) = fixture()?;
    let mut source = editor.document.source();
    let mut entity = EntityData::new("新角色");
    let id = entity.id;
    entity.transform.translation = [0., 3., 0.];
    source.base.entities.push(entity);
    editor.document = Document::open_source(source)?;
    let mut body = PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    };
    editor.edit_physics(
        id,
        Some(body.clone()),
        Some(Character::default()),
        &mut assets,
    )?;
    editor.commit()?;
    for radius in [0.4, 0.5, 0.6] {
        body.shape = ColliderShape::Capsule {
            half_height: 0.5,
            radius,
        };
        editor.edit_physics(
            id,
            Some(body.clone()),
            Some(Character::default()),
            &mut assets,
        )?;
    }
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 2);
    let configured = editor.document.shared_source();
    let mut invalid = body;
    invalid.motion = BodyMotion::Dynamic;
    assert!(
        editor
            .edit_physics(id, Some(invalid), Some(Character::default()), &mut assets)
            .is_err()
    );
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.shared_source(), configured);
    editor.save(&mut assets)?;
    let (mut reopened, mut rebuilt) = open(
        temp.path().join("project"),
        temp.path().join("rebuilt"),
        "main.scene".into(),
    )?;
    assert_eq!(reopened.document.shared_source(), configured);
    reopened.start(&mut rebuilt)?;
    let world = reopened.world();
    assert!(reopened.edit_physics(id, None, None, &mut rebuilt).is_err());
    assert_eq!(reopened.world(), world);
    assert!(!reopened.document.is_editing());
    let play = reopened.play.as_mut().unwrap();
    for _ in 0..30 {
        play.advance(1. / 60.)?;
    }
    assert!(play.character(id).unwrap().position[1] < 3.);
    reopened.stop();
    assert_eq!(reopened.document.shared_source(), configured);
    assert!(!reopened.document.is_dirty());
    editor.edit_physics(id, None, None, &mut assets)?;
    editor.commit()?;
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), configured);
    assert!(!editor.document.is_dirty());
    Ok(())
}
#[test]
fn model_edit_save_reopen_history_and_play_stop_isolation() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let snapshot = editor.snapshot(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    for x in [0.2, 0.5, 0.8] {
        editor.edit(
            id,
            schema::TRANSLATION,
            FieldValue::Vec3([x, 0.7, 0.]),
            &mut assets,
        )?;
    }
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    let edited = editor.snapshot(&mut assets)?;
    editor.document.undo()?;
    assert_eq!(editor.snapshot(&mut assets)?, snapshot);
    editor.document.redo()?;
    assert_eq!(editor.snapshot(&mut assets)?, edited);
    editor.save(&mut assets)?;
    assert!(!editor.document.is_dirty());
    let saved = editor.document.source();
    assert_eq!(Document::load(&editor.source)?.source(), saved);
    let history = editor.document.history_len();
    editor.start(&mut assets)?;
    let play_world = editor.world();
    assert_ne!(play_world, editor.document.world_id());
    editor.play.as_mut().unwrap().set_paused(true);
    editor.play.as_mut().unwrap().advance(10.)?;
    assert_eq!(editor.play.as_ref().unwrap().steps(), 0);
    editor.play.as_mut().unwrap().single_step()?;
    assert_eq!(editor.play.as_ref().unwrap().steps(), 1);
    editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3([9., 8., 7.]),
        &mut assets,
    )?;
    assert_ne!(editor.snapshot(&mut assets)?, edited);
    editor.stop();
    assert_eq!(editor.snapshot(&mut assets)?, edited);
    assert_eq!(editor.document.source(), saved);
    assert_eq!(editor.document.history_len(), history);
    assert!(!editor.document.is_dirty());
    editor.start(&mut assets)?;
    assert_ne!(editor.world(), play_world);
    Ok(())
}
#[test]
fn invalid_play_candidate_keeps_the_edit_owner_and_history() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let snapshot = editor.snapshot(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|e| e.model.is_some())
        .unwrap()
        .id;
    editor.edit(
        id,
        schema::MESH,
        FieldValue::Asset(comet_core::AssetId::new()),
        &mut assets,
    )?;
    editor.commit()?;
    let source = editor.document.source();
    let history = editor.document.history_len();
    assert!(editor.start(&mut assets).is_err());
    assert!(editor.play.is_none());
    assert_eq!(editor.world(), editor.document.world_id());
    assert_eq!(editor.document.source(), source);
    assert_eq!(editor.document.history_len(), history);
    editor.document.undo()?;
    editor.start(&mut assets)?;
    Ok(())
}
#[test]
fn stale_world_target_and_removed_entity_pick_results_are_rejected() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.last_snapshot = editor.shared_snapshot(&mut assets)?;
    let entity = editor.last_snapshot.entities[0].id;
    editor.target_generation = 7;
    let mut feedback = FrameFeedback {
        number: 1,
        token: ViewToken {
            world: editor.world(),
            view: 1,
            target_generation: 7,
        },
        presented: true,
        backend: "test".into(),
        worker_thread: "test".into(),
        draw_count: 5,
        gpu_ms: 0.,
        pick: None,
    };
    for token in [
        ViewToken {
            world: WorldId::new(),
            ..feedback.token
        },
        ViewToken {
            target_generation: 6,
            ..feedback.token
        },
    ] {
        feedback.pick = Some(PickFeedback {
            token,
            entity: Some(entity),
        });
        editor.feedback(&feedback);
        assert!(editor.selected.is_none());
    }
    feedback.pick = Some(PickFeedback {
        token: feedback.token,
        entity: Some(EntityId::new()),
    });
    editor.feedback(&feedback);
    assert!(editor.selected.is_none());
    feedback.pick = Some(PickFeedback {
        token: feedback.token,
        entity: Some(entity),
    });
    editor.feedback(&feedback);
    assert_eq!(editor.selected, Some(entity));
    Ok(())
}
#[test]
fn author_scene_paths_cannot_escape_and_save_rechecks_the_current_parent() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let project = temp.path().join("project");
    let outside = temp.path().join("outside.scene");
    std::fs::write(&outside, editor.document.source().to_json()?)?;
    let bytes = std::fs::read(&outside)?;
    assert!(
        open(
            project.clone(),
            temp.path().join("bad-products"),
            "../outside.scene".into()
        )
        .is_err()
    );
    assert!(open(project, temp.path().join("bad-products"), outside.clone()).is_err());
    editor.source = outside.clone();
    assert!(editor.save(&mut assets).is_err());
    assert_eq!(std::fs::read(outside)?, bytes);
    Ok(())
}
#[test]
fn expanded_snapshot_cache_tracks_document_products_and_play_worlds() -> Result<()> {
    use std::sync::Arc;
    let (temp, mut editor, mut assets) = fixture()?;
    let before = editor.shared_snapshot(&mut assets)?;
    for _ in 0..100 {
        assert!(Arc::ptr_eq(&before, &editor.shared_snapshot(&mut assets)?));
    }
    let source = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    gltf["nodes"][1]["translation"][0] = serde_json::json!(-2.);
    std::fs::write(source, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(editor.paths.root(), &editor.products)?;
    assets.refresh()?;
    let reloaded = editor.shared_snapshot(&mut assets)?;
    assert!(!Arc::ptr_eq(&before, &reloaded));
    assert_ne!(*before, *reloaded);
    editor.start(&mut assets)?;
    let play = editor.shared_snapshot(&mut assets)?;
    assert!(!Arc::ptr_eq(&reloaded, &play));
    editor.stop();
    assert!(Arc::ptr_eq(
        &reloaded,
        &editor.shared_snapshot(&mut assets)?
    ));
    Ok(())
}

impl Editor {
    pub(crate) fn snapshot(&mut self, assets: &mut AssetStore) -> Result<SceneSnapshot> {
        Ok(self.shared_snapshot(assets)?.as_ref().clone())
    }
}
