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
