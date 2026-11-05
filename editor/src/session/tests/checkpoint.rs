use super::scene_requests::{bind_author, frame, publish, scene_with_script, write_asset};
use super::*;
use comet_application::CheckpointAction;
use comet_scene::Script;
use std::sync::Arc;

#[test]
fn preview_slot_tracks_the_running_scene_and_preserves_authoring_and_failed_candidates()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let b_script = write_asset(&editor, "checkpoint_b.rhai",
        b"fn on_start(ctx) { let old = ctx.progress(\"done\", false); ctx.set_hud(if old { \"restored B\" } else { \"fresh B\" }); ctx.set_progress(\"done\", true); }")?;
    let b_source = scene_with_script("B", b_script);
    let b_owner = b_source.base.entities[0].id;
    let b = write_asset(&editor, "checkpoint_b.scene", &b_source.to_json()?)?;
    let a_script = write_asset(
        &editor,
        "checkpoint_a.rhai",
        format!(
            "fn on_start(ctx) {{ ctx.set_progress(\"done\", true); ctx.request_scene(\"{b}\"); }}"
        )
        .as_bytes(),
    )?;
    publish(&editor, &mut assets)?;
    bind_author(&mut editor, &mut assets, Script::new(a_script))?;
    let author = editor.document.shared_source();
    let history = editor.document.history_len();
    let bytes = std::fs::read(&editor.source)?;
    editor.start(&mut assets)?;
    editor.process_scene_request(&mut assets);
    assert_eq!(editor.play.as_ref().unwrap().scene_asset(), Some(b));
    assert_eq!(
        editor
            .play
            .as_ref()
            .unwrap()
            .progress_snapshot()
            .entries
            .len(),
        2
    );
    editor.preview_checkpoint_action(CheckpointAction::Save, &mut assets)?;
    let saved = editor.preview_checkpoint.clone().unwrap();
    assert_eq!(saved.scene, b);
    let play = editor.play.as_mut().unwrap();
    play.set_paused(true);
    play.set_field(
        &[b_owner],
        comet_scene::schema::TRANSLATION,
        comet_scene::schema::FieldValue::Vec3([3.; 3]),
    )?;
    let world = play.world_id();
    let snapshot = play.shared_snapshot();
    let progress = play.progress_snapshot();
    editor.preview_checkpoint.as_mut().unwrap().scene = AssetId::new();
    let rejected = editor.preview_checkpoint.as_ref().unwrap().to_bytes()?;
    assert!(
        editor
            .preview_checkpoint_action(CheckpointAction::Continue, &mut assets)
            .is_err()
    );
    assert_eq!(editor.world(), world);
    assert!(Arc::ptr_eq(
        &snapshot,
        &editor.play.as_mut().unwrap().shared_snapshot()
    ));
    assert!(Arc::ptr_eq(
        &progress,
        &editor.play.as_ref().unwrap().progress_snapshot()
    ));
    assert_eq!(
        editor.preview_checkpoint.as_ref().unwrap().to_bytes()?,
        rejected
    );
    editor.preview_checkpoint = Some(saved);
    editor.preview_checkpoint_action(CheckpointAction::Continue, &mut assets)?;
    assert_ne!(editor.world(), world);
    let play = editor.play.as_mut().unwrap();
    assert!(play.is_paused());
    assert_eq!(play.scene_asset(), Some(b));
    assert_eq!(
        play.hud_frame().unwrap().labels[0].text.as_ref(),
        "restored B"
    );
    assert_eq!(
        play.shared_snapshot().entities[0].transform.translation,
        [0.; 3]
    );
    assert!(Arc::ptr_eq(&author, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), history);
    assert_eq!(std::fs::read(&editor.source)?, bytes);
    editor.stop();
    assert!(editor.preview_checkpoint.is_none());
    editor.start(&mut assets)?;
    editor.process_scene_request(&mut assets);
    assert_eq!(
        editor.play.as_ref().unwrap().hud_frame().unwrap().labels[0]
            .text
            .as_ref(),
        "fresh B"
    );
    Ok(())
}

#[test]
fn preview_shortcuts_observe_late_modals_and_identity_changes_cannot_stop_or_export_the_old_run()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.start(&mut assets)?;
    editor.play.as_mut().unwrap().set_paused(true);
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let key = |key, pressed, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    frame(&ctx, &mut editor, &mut assets, 0, true, vec![])?;
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        true,
        vec![
            key(egui::Key::F5, true, Default::default()),
            key(egui::Key::O, true, egui::Modifiers::COMMAND),
        ],
    )?;
    assert!(editor.documents.is_blocking());
    assert!(editor.preview_checkpoint.is_none());
    editor.documents = Default::default();
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        true,
        vec![
            key(egui::Key::O, false, Default::default()),
            key(egui::Key::F5, false, Default::default()),
        ],
    )?;
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        true,
        vec![key(egui::Key::F5, true, Default::default())],
    )?;
    assert!(editor.preview_checkpoint.is_some());
    let world = editor.world();
    let slot = editor.preview_checkpoint.as_ref().unwrap().to_bytes()?;
    let source = editor.document.shared_source();
    let bytes = std::fs::read(&editor.source)?;
    let path = editor.paths.root().join("project.json");
    let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    settings["game_id"] = serde_json::json!(comet_core::GameId::new());
    std::fs::write(&path, serde_json::to_vec(&settings)?)?;
    let entry = editor.play.as_ref().unwrap().scene_asset().unwrap();
    let result = editor.begin_export(
        &mut assets,
        crate::export::ExportAction {
            entry,
            additional: vec![],
            output: editor.paths.root().join("must-not-export"),
            run: false,
        },
    );
    assert!(result.unwrap_err().to_string().contains("游戏身份已改变"));
    assert_eq!(editor.world(), world);
    assert_eq!(
        editor.preview_checkpoint.as_ref().unwrap().to_bytes()?,
        slot
    );
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    assert_eq!(std::fs::read(&editor.source)?, bytes);
    Ok(())
}
