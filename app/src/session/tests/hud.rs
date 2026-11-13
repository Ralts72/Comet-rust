use super::*;

#[test]
fn final_scene_views_and_shutdown_reports_use_only_the_current_world_hud() -> Result<()> {
    let a = AssetId::new();
    let b = AssetId::new();
    let program_a = AssetId::new();
    let program_b = AssetId::new();
    let first = entity("first", Some(program_a));
    let second = entity("second", Some(program_b));
    let second_id = serde_json::from_value::<EntityId>(second["id"].clone())?;
    let (_products, mut assets) = Products::new(vec![
        (a, scene(vec![first])?),
        (b, scene(vec![second])?),
        (
            program_a,
            script(format!(
                "fn on_start(ctx) {{ ctx.set_hud(\"world A\"); ctx.request_scene(\"{b}\"); }}"
            )),
        ),
        (
            program_b,
            script("fn on_start(ctx) { ctx.set_hud(\"world B\"); }"),
        ),
    ])?;
    let mut app = app(&mut assets, a, None)?;
    let old = frame_with_activity(&mut app, &mut assets, 0, false)?;
    assert_eq!(old.hud.as_ref().unwrap().world, old.world);
    assert_eq!(old.hud.as_ref().unwrap().labels[0].text.as_ref(), "world A");
    let current = frame(&mut app, &mut assets, 1)?;
    let hud = current.hud.as_ref().unwrap();
    assert_ne!(current.world, old.world);
    assert_eq!(hud.world, current.world);
    assert_eq!(hud.labels.len(), 1);
    assert_eq!(hud.labels[0].owner, second_id);
    assert_eq!(hud.labels[0].text.as_ref(), "world B");
    assert_eq!(old.hud.as_ref().unwrap().labels[0].text.as_ref(), "world A");
    let report = app.report();
    assert_eq!(report["hud"]["world"], json!(current.world));
    assert_eq!(
        report["hud"]["labels"][0],
        json!({"owner":second_id,"text":"world B"})
    );
    assert!(
        app.play.hud_frame().is_none(),
        "report captures HUD before its existing stop callback"
    );
    assert!(app.report()["hud"].is_null());
    Ok(())
}

#[test]
fn hud_views_preserve_primary_and_interact_input_authorization() -> Result<()> {
    let entry = AssetId::new();
    let program = AssetId::new();
    let (_products, mut assets) = Products::new(vec![
        (entry, scene(vec![entity("counter", Some(program))])?),
        (
            program,
            script(
                r#"fn on_start(ctx) { this.e = 0; this.primary = 0; ctx.set_hud("0/0"); }
            fn on_fixed_update(ctx, dt) {
                if ctx.pressed("interact") { this.e += 1; }
                if ctx.pressed("primary") { this.primary += 1; }
                ctx.set_hud(this.e.to_string() + "/" + this.primary.to_string());
            }"#,
            ),
        ),
    ])?;
    let mut app = app(&mut assets, entry, None)?;
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let pointer = egui::pos2(30., 50.);
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
    let mut number = 0;
    let mut draw = |app: &mut App, active, events| -> Result<ViewRequest> {
        let mut view = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 480.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                view = Some(app.frame(
                    ui,
                    &mut assets,
                    &FrameContext {
                        gamepad: Default::default(),
                        number,
                        dt: comet_runtime::FIXED_DELTA,
                        active,
                        activity_changed: false,
                        target_generation: 1,
                        size: [640, 480],
                        smoke: false,
                    },
                ));
            },
        );
        number += 1;
        output.textures_delta.clear();
        view.unwrap()
    };
    draw(&mut app, true, vec![])?;
    let pressed = draw(&mut app, true, events(true))?;
    assert_eq!(pressed.hud.as_ref().unwrap().labels[0].text.as_ref(), "1/1");
    let held = draw(&mut app, true, vec![])?;
    assert_eq!(held.hud.as_ref().unwrap().labels[0].text.as_ref(), "1/1");
    draw(&mut app, false, events(false))?;
    let denied = draw(&mut app, false, events(true))?;
    assert_eq!(denied.hud.as_ref().unwrap().labels[0].text.as_ref(), "1/1");
    draw(&mut app, true, events(false))?;
    let fresh = draw(&mut app, true, events(true))?;
    assert_eq!(fresh.hud.as_ref().unwrap().labels[0].text.as_ref(), "2/2");
    assert!(
        app.play
            .script_diagnostics()
            .iter()
            .all(|entry| !entry.failed)
    );
    Ok(())
}
