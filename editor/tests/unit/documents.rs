use super::*;
use comet_application::Application;
use comet_scene::{schema, schema::FieldValue};

fn preview(editor: &mut Editor, assets: &mut AssetStore) -> anyhow::Result<()> {
    let id = editor
        .snapshot(assets)?
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3([2., 3., 4.]),
        assets,
    )
}

#[test]
fn cancel_preserves_a_live_preview_and_discard_never_writes_it() -> anyhow::Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let saved = std::fs::read(&editor.source)?;
    preview(&mut editor, &mut assets)?;
    let edited = editor.document.shared_source();
    assert!(!editor.request_close());
    assert!(
        !editor.request_close(),
        "repeated close must not bypass confirmation"
    );
    editor.resolve_transition(Choice::Cancel, &mut assets);
    assert!(!editor.close_ready());
    assert!(editor.document.is_editing());
    assert_eq!(editor.document.shared_source(), edited);
    assert!(!editor.request_close());
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert!(editor.close_ready());
    assert_eq!(std::fs::read(&editor.source)?, saved);
    Ok(())
}

#[test]
fn save_failure_stays_open_and_retry_persists_authoring_after_play() -> anyhow::Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    preview(&mut editor, &mut assets)?;
    editor.start(&mut assets)?;
    let authoring = editor.document.shared_source();
    let source = editor.source.clone();
    let backup = source.with_extension("backup");
    std::fs::rename(&source, &backup)?;
    std::fs::create_dir(&source)?;
    assert!(!editor.request_close());
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(!editor.close_ready());
    assert!(editor.documents.pending && editor.documents.error.is_some());
    assert!(editor.document.is_dirty());
    assert!(editor.play.is_none());
    assert_eq!(editor.document.shared_source(), authoring);
    std::fs::remove_dir(&source)?;
    std::fs::rename(&backup, &source)?;
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(editor.close_ready());
    assert!(!editor.document.is_dirty());
    let reopened = comet_editor_core::Document::load(&source)?;
    assert_eq!(reopened.shared_source(), authoring);
    Ok(())
}

#[test]
fn unchanged_authoring_closes_directly_during_play() -> anyhow::Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    editor.start(&mut assets)?;
    assert!(editor.request_close());
    assert!(editor.close_ready());
    Ok(())
}

#[test]
fn scene_creation_can_be_cancelled_and_switching_saves_the_current_authoring() -> anyhow::Result<()>
{
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let original = editor.source.clone();
    editor.export.template_error = Some("preserve template configuration".into());
    preview(&mut editor, &mut assets)?;
    let edited = editor.document.shared_source();
    editor.request_document_action(DocumentAction::New("new.scene".into()));
    editor.resolve_transition(Choice::Cancel, &mut assets);
    assert!(editor.document.is_editing());
    assert!(!original.parent().unwrap().join("new.scene").exists());
    editor.request_document_action(DocumentAction::New("new.scene".into()));
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(editor.documents.error.is_none());
    assert!(editor.source.ends_with("new.scene"));
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_dirty());
    assert!(editor.document.source().base.entities.is_empty());
    assert_eq!(
        comet_editor_core::Document::load(&original)?.shared_source(),
        edited
    );
    let metadata: comet_asset_pipeline::SourceMeta = serde_json::from_slice(&std::fs::read(
        original.parent().unwrap().join("new.scene.meta"),
    )?)?;
    assert_eq!(editor.export.entry_scene(), metadata.id);
    assert_eq!(
        editor
            .sources
            .of_kind(comet_asset_data::product::AssetKind::Scene)
            .count(),
        2
    );
    assert_eq!(
        editor.export.template_error.as_deref(),
        Some("preserve template configuration")
    );
    editor.request_document_action(DocumentAction::Open(original.clone()));
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert_eq!(editor.source, original);
    assert_eq!(editor.document.shared_source(), edited);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn failed_open_keeps_current_play_document_and_product_store() -> anyhow::Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    preview(&mut editor, &mut assets)?;
    editor.start(&mut assets)?;
    let source = editor.source.clone();
    let content = editor.document.shared_source();
    let world = editor.world();
    let revision = assets.revision().to_owned();
    let bad = source.parent().unwrap().join("broken.scene");
    std::fs::write(&bad, b"not a scene")?;
    for path in [bad, "../outside.scene".into()] {
        editor.request_document_action(DocumentAction::Open(path));
        editor.resolve_transition(Choice::Discard, &mut assets);
        assert!(editor.documents.error.is_some());
        assert_eq!(editor.source, source);
        assert_eq!(editor.world(), world);
        assert_eq!(editor.document.shared_source(), content);
        assert_eq!(assets.revision(), revision);
        assert!(editor.document.is_dirty());
        editor.resolve_transition(Choice::Cancel, &mut assets);
    }
    assert!(!editor.close_ready());
    Ok(())
}

#[test]
fn save_failure_prevents_a_new_scene_from_being_created() -> anyhow::Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    preview(&mut editor, &mut assets)?;
    let source = editor.source.clone();
    let backup = source.with_extension("backup");
    std::fs::rename(&source, &backup)?;
    std::fs::create_dir(&source)?;
    editor.request_document_action(DocumentAction::New("new.scene".into()));
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(editor.documents.error.is_some());
    assert!(editor.document.is_dirty());
    assert_eq!(editor.source, source);
    assert!(!source.parent().unwrap().join("new.scene").exists());
    assert!(!source.parent().unwrap().join("new.scene.meta").exists());
    Ok(())
}
