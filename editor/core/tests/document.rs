use anyhow::Result;
use comet_editor_core::Document;
use comet_scene::{
    EntityData, SceneSnapshot,
    schema::{self, FieldValue},
};
use std::fs;
#[test]
fn preview_history_savepoint_and_failed_save() -> Result<()> {
    let a = EntityData::new("editable");
    let id = a.id;
    let scene = SceneSnapshot {
        version: 1,
        entities: vec![a],
    };
    let mut doc = Document::open(scene.clone())?;
    assert!(!doc.is_dirty());
    doc.begin()?;
    for x in [1., 2., 3.] {
        doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([x, 0., 0.]))?;
    }
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("main.scene");
    assert!(doc.save(&file).is_err());
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    assert!(doc.is_dirty());
    doc.undo()?;
    assert!(!doc.is_dirty());
    doc.redo()?;
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, b"file")?;
    assert!(doc.save(&blocker.join("cannot-save.scene")).is_err());
    assert!(doc.is_dirty());
    doc.save(&file)?;
    assert!(!doc.is_dirty());
    assert_eq!(Document::load(&file)?.snapshot(), doc.snapshot());
    doc.begin()?;
    doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([8.; 3]))?;
    doc.cancel()?;
    assert!(!doc.is_dirty());
    assert_eq!(doc.history_len(), 1);
    doc.undo()?;
    assert!(doc.is_dirty());
    doc.redo()?;
    assert!(!doc.is_dirty());
    doc.begin()?;
    doc.commit()?;
    assert_eq!(doc.history_len(), 1);
    Ok(())
}
#[test]
fn clone_is_one_undo_step_and_new_edit_discards_redo() -> Result<()> {
    let a = EntityData::new("root");
    let id = a.id;
    let mut doc = Document::open(SceneSnapshot {
        version: 1,
        entities: vec![a],
    })?;
    doc.clone_subtrees(&[id])?;
    assert_eq!(doc.snapshot().entities.len(), 2);
    assert_eq!(doc.history_len(), 1);
    doc.undo()?;
    doc.begin()?;
    doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([2.; 3]))?;
    doc.commit()?;
    assert!(doc.redo().is_err());
    assert_eq!(doc.snapshot().entities.len(), 1);
    Ok(())
}

#[test]
fn preview_dirty_state_and_shared_reads_follow_content_changes_only() -> Result<()> {
    use std::sync::Arc;
    let e = EntityData::new("editable");
    let id = e.id;
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![e],
        ..Default::default()
    })?;
    let source = doc.shared_source();
    let snapshot = doc.shared_snapshot();
    let revision = doc.revision();
    for _ in 0..1000 {
        assert!(!doc.is_dirty());
        assert!(Arc::ptr_eq(&source, &doc.shared_source()));
        assert!(Arc::ptr_eq(&snapshot, &doc.shared_snapshot()));
    }
    assert_eq!(doc.revision(), revision);
    doc.begin()?;
    assert!(!doc.is_dirty());
    doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([1.; 3]))?;
    assert!(doc.is_dirty());
    assert!(!Arc::ptr_eq(&source, &doc.shared_source()));
    doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([0.; 3]))?;
    assert!(!doc.is_dirty());
    doc.commit()?;
    assert_eq!(doc.history_len(), 0);
    doc.begin()?;
    doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([2.; 3]))?;
    doc.cancel()?;
    assert!(!doc.is_dirty());
    assert_eq!(doc.source(), *source);
    Ok(())
}
#[test]
fn bounded_history_preserves_savepoints_and_does_not_reuse_branch_state_ids() -> Result<()> {
    let e = EntityData::new("editable");
    let id = e.id;
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![e],
        ..Default::default()
    })?;
    let set = |doc: &mut Document, x: f32| -> Result<()> {
        doc.begin()?;
        doc.preview_field(&[id], schema::TRANSLATION, FieldValue::Vec3([x, 0., 0.]))?;
        doc.commit()
    };
    for x in 1..=300 {
        set(&mut doc, x as f32)?;
    }
    assert_eq!(doc.history_len(), 256);
    let temp = tempfile::tempdir()?;
    doc.save(&temp.path().join("main.scene"))?;
    let saved = doc.state_id();
    doc.undo()?;
    assert!(doc.is_dirty());
    doc.redo()?;
    assert_eq!(doc.state_id(), saved);
    assert!(!doc.is_dirty());
    doc.undo()?;
    set(&mut doc, 999.)?;
    assert_ne!(doc.state_id(), saved);
    assert!(doc.redo().is_err());
    for _ in 0..256 {
        doc.undo()?;
    }
    assert!(doc.undo().is_err());
    assert!(doc.is_dirty());
    Ok(())
}
