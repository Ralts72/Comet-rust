use super::*;
use comet_editor_core::Document;
use comet_scene::Script;

fn fixture() -> Result<(tempfile::TempDir, Editor, AssetStore)> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let path = editor.project_root().join("assets/hud.rhai");
    std::fs::write(
        &path,
        r#"fn on_start(ctx) { this.e = 0; this.primary = 0; ctx.set_hud("0/0"); }
        fn on_fixed_update(ctx, dt) {
            if ctx.pressed("interact") { this.e += 1; }
            if ctx.pressed("primary") { this.primary += 1; }
            ctx.set_hud(this.e.to_string() + "/" + this.primary.to_string());
        }"#,
    )?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let mut entity = EntityData::new("HUD owner");
    entity.script = Some(Script::new(
        comet_asset_pipeline::SourceMeta::load(&path)?.id,
    ));
    editor.document = Document::open(SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    })?;
    editor.save(&mut assets)?;
    editor.start(&mut assets)?;
    Ok((temp, editor, assets))
}

fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    number: u64,
    events: Vec<egui::Event>,
) -> Result<ViewRequest> {
    frame_with_pad(ctx, editor, assets, number, events, Default::default())
}
fn frame_with_pad(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    number: u64,
    events: Vec<egui::Event>,
    gamepad: comet_application::GamepadFrame,
) -> Result<ViewRequest> {
    let mut view = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            view = Some(editor.frame(
                ui,
                assets,
                &FrameContext {
                    gamepad: gamepad.clone(),
                    number,
                    dt: comet_runtime::FIXED_DELTA,
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
    view.unwrap()
}

#[test]
fn play_hud_keeps_primary_and_interact_edges_and_document_modals_keep_their_authorization()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let initial = frame(&ctx, &mut editor, &mut assets, 0, vec![])?;
    assert_eq!(initial.hud.as_ref().unwrap().world, initial.world);
    let pointer = initial.viewport.left_top() + egui::vec2(32., 64.);
    let events = |pressed| {
        vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
            egui::Event::Key {
                key: egui::Key::E,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            },
        ]
    };
    let pressed = frame(&ctx, &mut editor, &mut assets, 1, events(true))?;
    assert_eq!(pressed.hud.as_ref().unwrap().labels[0].text.as_ref(), "1/1");
    let held = frame(&ctx, &mut editor, &mut assets, 2, vec![])?;
    assert_eq!(held.hud.as_ref().unwrap().labels[0].text.as_ref(), "1/1");
    frame(&ctx, &mut editor, &mut assets, 3, events(false))?;
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    let steps = editor.play.as_ref().unwrap().steps();
    let blocked = frame(&ctx, &mut editor, &mut assets, 4, events(true))?;
    assert_eq!(blocked.hud.as_ref().unwrap().labels[0].text.as_ref(), "1/1");
    assert_eq!(editor.play.as_ref().unwrap().steps(), steps);
    editor.documents = Default::default();
    frame(&ctx, &mut editor, &mut assets, 5, events(false))?;
    let fresh = frame(&ctx, &mut editor, &mut assets, 6, events(true))?;
    assert_eq!(fresh.hud.as_ref().unwrap().labels[0].text.as_ref(), "2/2");
    assert!(
        editor
            .play
            .as_ref()
            .unwrap()
            .script_diagnostics()
            .iter()
            .all(|entry| !entry.failed)
    );
    Ok(())
}

#[test]
fn view_recovery_discards_play_hud_without_changing_authoring_or_its_saved_file() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let author = editor.document.shared_source();
    let bytes = std::fs::read(&editor.source)?;
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let live = frame(&ctx, &mut editor, &mut assets, 0, vec![])?;
    assert!(live.hud.is_some());
    let fallback = editor.view_error(anyhow::anyhow!("HUD recovery content failure"))?;
    assert_ne!(fallback.world, live.world);
    assert_eq!(fallback.world, editor.document.world_id());
    assert!(fallback.hud.is_none());
    assert!(editor.play.is_none());
    assert!(editor.report()["hud"].is_null());
    assert!(std::sync::Arc::ptr_eq(
        &author,
        &editor.document.shared_source()
    ));
    assert_eq!(std::fs::read(&editor.source)?, bytes);
    Ok(())
}

#[test]
fn gamepad_play_merges_actions_moves_the_character_and_isolates_pause_and_late_modals() -> Result<()>
{
    use comet_application::GamepadFrame;
    use comet_core::{Action, ActionEvent};
    use comet_scene::{BodyMotion, ColliderShape};
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.stop();
    let mut source = editor.document.source();
    let mut actor = EntityData::new("手柄角色");
    actor.transform.translation = [0., 2., 0.];
    actor.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    actor.character = Some(Character::default());
    source.base.entities.push(actor);
    editor.document = Document::open_source(source)?;
    editor.save(&mut assets)?;
    editor.start(&mut assets)?;
    let author = editor.document.shared_source();
    let bytes = std::fs::read(&editor.source)?;
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![])?;
    let before = editor
        .play
        .as_ref()
        .unwrap()
        .player_character()
        .unwrap()
        .position;
    let camera = editor.camera;
    let mut pad = GamepadFrame {
        look_rate: [90., 0.],
        ..Default::default()
    };
    pad.actions.enabled = true;
    pad.actions.movement = [0., 1.];
    for action in [Action::Interact, Action::Primary] {
        pad.actions.held.set(action, true);
        pad.actions.events.push(ActionEvent {
            action,
            pressed: true,
        });
    }
    let view = frame_with_pad(&ctx, &mut editor, &mut assets, 1, vec![], pad.clone())?;
    assert_eq!(view.hud.unwrap().labels[0].text.as_ref(), "1/1");
    assert_ne!(editor.camera, camera);
    let after = editor
        .play
        .as_ref()
        .unwrap()
        .player_character()
        .unwrap()
        .position;
    assert!((after[0] - before[0]).hypot(after[2] - before[2]) > 0.);
    let keys = |pressed| {
        [egui::Key::E, egui::Key::J]
            .map(|key| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            })
            .to_vec()
    };
    pad.actions.events.clear();
    let view = frame_with_pad(&ctx, &mut editor, &mut assets, 2, keys(true), pad.clone())?;
    assert_eq!(view.hud.unwrap().labels[0].text.as_ref(), "1/1");
    for action in [Action::Interact, Action::Primary] {
        pad.actions.held.set(action, false);
        pad.actions.events.push(ActionEvent {
            action,
            pressed: false,
        });
    }
    frame_with_pad(&ctx, &mut editor, &mut assets, 3, vec![], pad.clone())?;
    frame(&ctx, &mut editor, &mut assets, 4, keys(false))?;
    let mut pause = GamepadFrame {
        pause_down: true,
        pause_pressed: true,
        ..Default::default()
    };
    let steps = editor.play.as_ref().unwrap().steps();
    frame_with_pad(&ctx, &mut editor, &mut assets, 5, vec![], pause.clone())?;
    assert!(editor.play.as_ref().unwrap().is_paused());
    assert_eq!(editor.play.as_ref().unwrap().steps(), steps);
    pause.pause_pressed = false;
    frame_with_pad(&ctx, &mut editor, &mut assets, 6, vec![], pause.clone())?;
    assert!(editor.play.as_ref().unwrap().is_paused());
    frame(&ctx, &mut editor, &mut assets, 7, vec![])?;
    pause.pause_pressed = true;
    frame_with_pad(&ctx, &mut editor, &mut assets, 8, vec![], pause.clone())?;
    assert!(!editor.play.as_ref().unwrap().is_paused());
    assert_eq!(editor.play.as_ref().unwrap().steps(), steps);
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    let camera = editor.camera;
    pad.pause_down = true;
    pad.pause_pressed = true;
    frame_with_pad(&ctx, &mut editor, &mut assets, 9, vec![], pad.clone())?;
    assert_eq!(editor.camera, camera);
    assert_eq!(editor.play.as_ref().unwrap().steps(), steps);
    assert!(!editor.play.as_ref().unwrap().is_paused());
    editor.documents = Default::default();
    pad.pause_pressed = false;
    frame_with_pad(&ctx, &mut editor, &mut assets, 10, vec![], pad)?;
    assert!(!editor.play.as_ref().unwrap().is_paused());
    editor.stop();
    assert_eq!(editor.document.shared_source(), author);
    assert_eq!(std::fs::read(&editor.source)?, bytes);
    Ok(())
}
