use super::*;
use comet_scene::{EntityData, EntityRef, ScriptValue};
use std::fs;

fn script_asset(editor: &Editor, assets: &mut AssetStore) -> Result<comet_core::AssetId> {
    let source = editor.project_root().join("assets/test.rhai");
    fs::write(&source, "fn on_fixed_update(ctx, dt) {}")?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    Ok(comet_asset_pipeline::SourceMeta::load(&source)?.id)
}

#[test]
fn script_configuration_preserves_types_history_save_and_play_isolation() -> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let asset = script_asset(&editor, &mut assets)?;
    let entity = EntityData::new("script owner");
    let id = entity.id;
    editor.document.insert_entity(entity, &mut assets)?;
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
    let mut script = Script::new(asset);
    script.parameters.extend([
        ("enabled".into(), ScriptValue::Bool(true)),
        ("count".into(), ScriptValue::Integer(12)),
        ("speed".into(), ScriptValue::Number(1.25)),
        ("name".into(), ScriptValue::Text("角色".into())),
        ("offset".into(), ScriptValue::Vec3([1., 2., 3.])),
        (
            "target".into(),
            ScriptValue::Entity(Some(EntityRef::local(id))),
        ),
    ]);
    for speed in [2., 3., 4.] {
        script
            .parameters
            .insert("speed".into(), ScriptValue::Number(speed));
        editor.edit_script(id, Some(script.clone()), &mut assets)?;
    }
    editor.commit()?;
    assert_eq!(editor.document.history_len(), history + 1);
    let configured = editor.document.shared_source();
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), configured);
    editor.save(&mut assets)?;
    assert_eq!(
        comet_editor_core::Document::load(&editor.source)?.shared_source(),
        configured
    );
    editor.start(&mut assets)?;
    let world = editor.world();
    assert!(editor.edit_script(id, None, &mut assets).is_err());
    assert_eq!(editor.world(), world);
    editor.stop();
    assert_eq!(editor.document.shared_source(), configured);
    editor.edit_script(id, None, &mut assets)?;
    editor.commit()?;
    assert!(
        editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .script
            .is_none()
    );
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), configured);
    Ok(())
}

#[test]
fn invalid_script_candidates_do_not_publish_or_leave_a_new_transaction_open() -> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let asset = script_asset(&editor, &mut assets)?;
    let entity = EntityData::new("script owner");
    let id = entity.id;
    editor.document.insert_entity(entity, &mut assets)?;
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
    let wrong_kind = assets
        .index()
        .products
        .iter()
        .find(|(_, entry)| entry.kind != comet_asset_data::product::AssetKind::Script)
        .unwrap()
        .0;
    assert!(
        editor
            .edit_script(id, Some(Script::new(*wrong_kind)), &mut assets)
            .is_err()
    );
    let mut invalid = Script::new(asset);
    invalid.parameters.insert(
        "target".into(),
        ScriptValue::Entity(Some(EntityRef::local(EntityId::new()))),
    );
    assert!(editor.edit_script(id, Some(invalid), &mut assets).is_err());
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), history);
    editor.edit_script(id, Some(Script::new(asset)), &mut assets)?;
    editor.cancel()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), history);
    Ok(())
}
