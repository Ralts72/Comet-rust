use super::*;
use comet_asset_pipeline::SourceMeta;
use std::{fs, sync::Arc};

#[test]
fn creating_a_prefab_cancels_or_saves_the_old_scene_before_opening_the_empty_author_document()
-> anyhow::Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let scene = editor.source.clone();
    preview(&mut editor, &mut assets)?;
    let authored = editor.document.shared_source();
    let path = scene.parent().unwrap().join("new.prefab");
    editor.request_document_action(DocumentAction::NewPrefab("new.prefab".into()));
    editor.resolve_transition(Choice::Cancel, &mut assets);
    assert!(!path.exists() && !SourceMeta::path(&path).exists());
    assert!(editor.document.is_editing());
    assert!(Arc::ptr_eq(&authored, &editor.document.shared_source()));

    editor.request_document_action(DocumentAction::NewPrefab("new.prefab".into()));
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(
        editor.documents.error.is_none(),
        "{:?}",
        editor.documents.error
    );
    assert_eq!(editor.source, path);
    assert_eq!(
        editor.document.prefab_asset(),
        Some(SourceMeta::load(&path)?.id)
    );
    assert!(editor.document.shared_source().base.entities.is_empty());
    assert!(!editor.document.is_dirty());
    assert_eq!(editor.document.history_len(), 0);
    assert_eq!(
        comet_editor_core::Document::load(&scene)?.shared_source(),
        authored
    );

    editor.create_object(None, &mut assets)?;
    let changed = editor.document.shared_source();
    editor.request_document_action(DocumentAction::Open(scene.clone()));
    editor.resolve_transition(Choice::Cancel, &mut assets);
    assert_eq!(editor.source, path);
    assert!(Arc::ptr_eq(&changed, &editor.document.shared_source()));
    editor.request_document_action(DocumentAction::Open(scene.clone()));
    editor.resolve_transition(Choice::Save, &mut assets);
    assert_eq!(editor.source, scene);
    assert!(editor.document.prefab_asset().is_none());
    assert_eq!(
        comet_editor_core::Document::load(&path)?.shared_source(),
        changed
    );
    Ok(())
}

#[test]
fn failed_save_and_creation_preserve_the_old_preview_world_and_existing_files() -> anyhow::Result<()>
{
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    preview(&mut editor, &mut assets)?;
    editor.start(&mut assets)?;
    let source = editor.source.clone();
    let world = editor.world();
    let content = editor.document.shared_source();
    let revision = assets.revision().to_owned();
    let destination = source.parent().unwrap().join("new.prefab");
    let backup = source.with_extension("backup");
    fs::rename(&source, &backup)?;
    fs::create_dir(&source)?;
    editor.request_document_action(DocumentAction::NewPrefab("new.prefab".into()));
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(editor.documents.error.is_some());
    assert!(!destination.exists() && !SourceMeta::path(&destination).exists());
    assert_eq!(editor.world(), world);
    assert!(Arc::ptr_eq(&content, &editor.document.shared_source()));
    fs::remove_dir(&source)?;
    fs::rename(backup, &source)?;

    for path in ["../outside.prefab", "wrong.scene"] {
        editor.request_document_action(DocumentAction::NewPrefab(path.into()));
        editor.resolve_transition(Choice::Discard, &mut assets);
        assert!(editor.documents.error.is_some());
        assert_eq!(editor.source, source);
        assert_eq!(editor.world(), world);
        assert!(Arc::ptr_eq(&content, &editor.document.shared_source()));
        assert_eq!(assets.revision(), revision);
    }

    editor.create_prefab_source(std::path::Path::new("new.prefab"))?;
    let bytes = fs::read(&destination)?;
    let metadata = fs::read(SourceMeta::path(&destination))?;
    editor.request_document_action(DocumentAction::NewPrefab("new.prefab".into()));
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert!(editor.documents.error.is_some());
    assert_eq!(fs::read(&destination)?, bytes);
    assert_eq!(fs::read(SourceMeta::path(&destination))?, metadata);
    assert_eq!(editor.source, source);
    assert_eq!(editor.world(), world);
    assert!(Arc::ptr_eq(&content, &editor.document.shared_source()));
    editor.resolve_transition(Choice::Cancel, &mut assets);
    Ok(())
}

#[test]
fn a_prefab_created_before_load_failure_retries_open_without_changing_the_published_identity()
-> anyhow::Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    editor.start(&mut assets)?;
    let source = editor.source.clone();
    let world = editor.world();
    let revision = assets.revision().to_owned();
    let broken = source.parent().unwrap().join("broken.scene");
    fs::write(&broken, b"broken scene prevents project preparation")?;
    editor.request_document_action(DocumentAction::NewPrefab("retry.prefab".into()));
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert!(editor.documents.error.is_some());
    let path = source.parent().unwrap().join("retry.prefab");
    let id = SourceMeta::load(&path)?.id;
    let bytes = fs::read(&path)?;
    let metadata = fs::read(SourceMeta::path(&path))?;
    assert_eq!(
        editor.documents.action,
        Some(DocumentAction::Open(path.clone()))
    );
    assert_eq!(editor.source, source);
    assert_eq!(editor.world(), world);
    assert_eq!(assets.revision(), revision);
    fs::remove_file(broken)?;
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert!(
        editor.documents.error.is_none(),
        "{:?}",
        editor.documents.error
    );
    assert_eq!(editor.source, path);
    assert_eq!(editor.document.prefab_asset(), Some(id));
    assert!(editor.play.is_none());
    assert_eq!(fs::read(&path)?, bytes);
    assert_eq!(fs::read(SourceMeta::path(&path))?, metadata);
    assert!(!editor.document.is_dirty());
    Ok(())
}
