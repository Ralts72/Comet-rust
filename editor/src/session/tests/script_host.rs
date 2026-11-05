use super::*;
use comet_scene::Script;

fn bind_script(editor: &mut Editor, assets: &mut AssetStore, source: &str) -> Result<PathBuf> {
    let path = editor.paths.assets().join("lifecycle.rhai");
    std::fs::write(&path, source)?;
    comet_asset_pipeline::prepare(editor.paths.root(), &editor.products)?;
    assets.refresh()?;
    let entity = EntityData::new("script host");
    let id = entity.id;
    editor.document.insert_entity(entity, assets)?;
    editor.edit_script(
        id,
        Some(Script::new(
            comet_asset_pipeline::SourceMeta::load(&path)?.id,
        )),
        assets,
    )?;
    editor.commit()?;
    Ok(path)
}

fn tick(editor: &mut Editor, assets: &mut AssetStore, number: u64) -> Result<()> {
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let mut result = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280., 800.),
            )),
            ..Default::default()
        },
        |ui| {
            result = Some(editor.frame(
                ui,
                assets,
                &FrameContext {
                    number,
                    dt: 1. / 60.,
                    active: true,
                    activity_changed: false,
                    target_generation: 1,
                    size: [1280, 800],
                    smoke: false,
                },
            ));
        },
    );
    output.textures_delta.clear();
    result.unwrap()?;
    Ok(())
}

#[test]
fn editor_collects_lifecycle_logs_and_bad_start_candidates_keep_the_old_play() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let path = bind_script(
        &mut editor,
        &mut assets,
        "fn on_start(ctx) { ctx.log(\"start\"); } fn on_fixed_update(ctx, dt) { ctx.log(\"tick\"); } fn on_stop(ctx) { ctx.log(\"stop\"); }",
    )?;
    let author = editor.document.shared_source();
    let history = editor.document.history_len();
    editor.start(&mut assets)?;
    let world = editor.world();
    assert_eq!(
        editor
            .console
            .entries()
            .iter()
            .filter(|entry| entry.message == "start")
            .count(),
        1
    );
    tick(&mut editor, &mut assets, 1)?;
    assert!(
        editor
            .console
            .entries()
            .iter()
            .any(|entry| entry.message == "tick"
                && entry.world == world
                && entry.phase == "on_fixed_update")
    );
    let count = editor.console.entries().len();
    editor.collect_script_diagnostics();
    assert_eq!(editor.console.entries().len(), count);
    std::fs::write(&path, "fn on_start(")?;
    comet_asset_pipeline::prepare(editor.paths.root(), &editor.products)?;
    assets.refresh()?;
    assert!(editor.start(&mut assets).is_err());
    assert_eq!(editor.world(), world);
    let steps = editor.play.as_ref().unwrap().steps();
    tick(&mut editor, &mut assets, 2)?;
    assert_eq!(editor.play.as_ref().unwrap().steps(), steps + 1);
    editor.stop();
    assert!(editor.play.is_none());
    assert_eq!(
        editor
            .console
            .entries()
            .iter()
            .filter(|entry| entry.message == "stop" && entry.world == world)
            .count(),
        1
    );
    let count = editor.console.entries().len();
    editor.stop();
    assert_eq!(editor.console.entries().len(), count);
    assert_eq!(editor.document.shared_source(), author);
    assert_eq!(editor.document.history_len(), history);
    assert_eq!(
        editor.report()["script_diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        count
    );
    let next = temp.path().join("next-project");
    let paths = comet_asset_pipeline::create_project(&next)?;
    let candidate = crate::project::ProjectCandidate::load(
        next,
        editor.product_locations.for_project(&paths)?,
        "main.scene".into(),
    )?;
    editor.accept_project_candidate(candidate, &mut assets)?;
    assert!(editor.console.entries().is_empty());
    Ok(())
}

#[test]
fn retained_history_checks_callback_signatures_without_executing_scripts() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let path = bind_script(
        &mut editor,
        &mut assets,
        "fn on_start(ctx) { throw \"validation must not execute callbacks\"; }",
    )?;
    let id = editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .find(|entity| entity.name == "script host")
        .unwrap()
        .id;
    editor.document.remove_subtrees(&[id], &mut assets)?;
    let retained: Vec<_> = editor.document.retained_sources().collect();
    crate::resources::validate_sources(&retained, &mut assets, editor.document.prefab_asset())?;
    assert!(editor.console.entries().is_empty());
    let accepted = assets.revision().to_owned();
    std::fs::write(path, "fn on_fixed_update(ctx) {}")?;
    comet_asset_pipeline::prepare(editor.paths.root(), &editor.products)?;
    let mut candidate = AssetStore::open(&editor.products)?;
    assert!(
        crate::resources::validate_sources(
            &retained,
            &mut candidate,
            editor.document.prefab_asset()
        )
        .is_err()
    );
    assert_eq!(assets.revision(), accepted);
    editor.document.undo()?;
    crate::resources::validate_sources(
        &[editor.document.shared_source()],
        &mut assets,
        editor.document.prefab_asset(),
    )?;
    Ok(())
}
