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
