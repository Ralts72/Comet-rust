use super::*;

#[test]
fn keyboard_restart_restores_play_baseline_and_preserves_authoring_and_pause() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.create_object(None, &mut assets)?;
    let id = editor.selection.active().unwrap();
    let authoring = editor.document.shared_source();
    let history = editor.document.history_len();
    editor.start(&mut assets)?;
    let world = editor.world();
    let baseline = editor.play.as_mut().unwrap().snapshot();
    editor.edit(
        id,
        comet_scene::schema::TRANSLATION,
        comet_scene::schema::FieldValue::Vec3([8., 3., 4.]),
        &mut assets,
    )?;
    editor.play.as_mut().unwrap().set_paused(true);
    assert_ne!(editor.play.as_mut().unwrap().snapshot(), baseline);
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        vec![egui::Event::Key {
            key: egui::Key::R,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_ne!(editor.world(), world);
    let play = editor.play.as_mut().unwrap();
    assert!(play.is_paused());
    assert_eq!(play.steps(), 0);
    assert_eq!(play.snapshot(), baseline);
    assert_eq!(editor.document.shared_source(), authoring);
    assert_eq!(editor.document.history_len(), history);
    let restarted = editor.world();
    frame(&ctx, &mut editor, &mut assets, 2, vec![]);
    assert_eq!(
        editor.world(),
        restarted,
        "held R must not repeatedly restart the world"
    );
    editor.restart_play()?;
    assert_ne!(editor.world(), restarted);
    assert_eq!(editor.document.shared_source(), authoring);
    editor.stop();
    assert!(editor.restart_play().is_err());
    assert_eq!(editor.document.shared_source(), authoring);
    Ok(())
}
