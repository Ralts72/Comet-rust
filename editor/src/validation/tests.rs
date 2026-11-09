use super::*;
use comet_asset_pipeline::{SourceMeta, create_project, prepare};
use comet_editor_core::Document;
use comet_scene::{EntityData, Script};
use std::{fs, sync::Arc};

#[test]
fn retained_history_checks_callback_signatures_without_executing_scripts() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let paths = create_project(&temp.path().join("project"))?;
    let products = temp.path().join("products");
    let script_path = paths.assets().join("validation.rhai");
    fs::write(
        &script_path,
        "fn on_start(ctx) { throw \"validation must not execute callbacks\"; }",
    )?;
    prepare(paths.root(), &products)?;
    let mut accepted = AssetStore::open(&products)?;
    let original_index = accepted.shared_index();
    let mut scripted = EntityData::new("history only");
    let id = scripted.id;
    scripted.script = Some(Script::new(SourceMeta::load(&script_path)?.id));
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![EntityData::new("current author"), scripted],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut document = Document::open_source(source)?;
    document.remove_subtrees(&[id], &mut accepted)?;
    let current = document.shared_source();
    let retained: Vec<_> = document.retained_sources().collect();
    let snapshot = validate_sources(
        retained.iter().map(|source| source.as_ref()),
        &mut accepted,
        None,
    )?;
    assert_eq!(
        snapshot, current.base,
        "return the current world, not a history snapshot"
    );
    assert!(validate_sources(std::iter::empty::<&ContentSource>(), &mut accepted, None).is_err());

    fs::write(&script_path, "fn on_fixed_update(ctx) {}")?;
    prepare(paths.root(), &products)?;
    let mut candidate = AssetStore::open(&products)?;
    validate_sources([current.as_ref()], &mut candidate, None)?;
    assert!(
        validate_sources(
            retained.iter().map(|source| source.as_ref()),
            &mut candidate,
            None,
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(&original_index, &accepted.shared_index()));
    document.undo()?;
    validate_sources([document.shared_source().as_ref()], &mut accepted, None)?;
    Ok(())
}
