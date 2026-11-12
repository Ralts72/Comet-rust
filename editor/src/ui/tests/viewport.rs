mod camera;
use super::*;
use comet_render::{FrameFeedback, PickFeedback, PickRequest, ViewToken};
use std::sync::Arc;

fn context(number: u64) -> FrameContext {
    FrameContext {
        number,
        dt: 1. / 60.,
        active: true,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    }
}

fn pointer(pos: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}

fn feedback(view: &ViewRequest, number: u64, entities: Vec<comet_core::EntityId>) -> FrameFeedback {
    let token = ViewToken {
        world: view.world,
        view: 1,
        target_generation: 1,
    };
    FrameFeedback {
        number,
        token,
        presented: true,
        backend: "test".into(),
        worker_thread: "test".into(),
        draw_count: 0,
        gpu_ms: 0.,
        pick: Some(PickFeedback { token, entities }),
    }
}

fn overlay(shape: &egui::epaint::Shape) -> bool {
    match shape {
        egui::epaint::Shape::Rect(rect) => {
            rect.fill == egui::Color32::from_rgba_unmultiplied(97, 174, 241, 32)
        }
        egui::epaint::Shape::Vec(shapes) => shapes.iter().any(overlay),
        _ => false,
    }
}

#[test]
fn rectangle_release_sorts_clamps_and_applies_one_deferred_batch_without_editing() -> Result<()> {
    for toggle in [false, true] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let ids: Vec<_> = editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .take(2)
            .map(|e| e.id)
            .collect();
        assert_eq!(ids.len(), 2);
        editor.selection.choose(Some(ids[0]), SelectMode::Replace);
        let original = editor.document.shared_source();
        let history = editor.document.history_len();
        let bytes = fs::read(&editor.source)?;
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        let initial = frame(&ctx, &mut editor, &mut assets, 0, vec![]);
        let start = initial.viewport.right_top() + Vec2::new(-25., 30.);
        let end = initial.viewport.left_bottom() + Vec2::new(-80., 80.);
        let pressed = frame(
            &ctx,
            &mut editor,
            &mut assets,
            1,
            vec![
                egui::Event::PointerMoved(start),
                pointer(start, true, egui::Modifiers::NONE),
            ],
        );
        assert!(pressed.pick.is_none());
        let (_, output) = frame_with_output(
            &ctx,
            &mut editor,
            &mut assets,
            context(2),
            vec![egui::Event::PointerMoved(end)],
        );
        assert!(output.shapes.iter().any(|shape| overlay(&shape.shape)));
        assert_eq!(editor.selection.ids(), vec![ids[0]]);
        let released = frame(
            &ctx,
            &mut editor,
            &mut assets,
            3,
            vec![pointer(
                end,
                false,
                if toggle {
                    egui::Modifiers::SHIFT
                } else {
                    egui::Modifiers::NONE
                },
            )],
        );
        let Some(PickRequest::Rect { min, max }) = released.pick else {
            panic!("missing rectangle request");
        };
        assert_eq!(min[0], 0.);
        assert_eq!(max[1], 1.);
        assert!(min[1] > 0. && max[0] < 1. && min[0] < max[0] && min[1] < max[1]);
        assert!(
            frame(&ctx, &mut editor, &mut assets, 4, vec![])
                .pick
                .is_none()
        );
        editor.feedback(&feedback(&released, 3, ids.clone()));
        if toggle {
            assert_eq!(editor.selection.ids(), vec![ids[1]]);
        } else {
            assert_eq!(editor.selection.len(), 2);
        }
        assert!(Arc::ptr_eq(&original, &editor.document.shared_source()));
        assert_eq!(editor.document.history_len(), history);
        assert_eq!(fs::read(&editor.source)?, bytes);
        assert!(!editor.document.is_editing());
    }
    Ok(())
}

#[test]
fn click_jitter_stays_a_point_and_a_drag_returning_to_zero_never_becomes_a_click() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let view = frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let start = view.viewport.left_top() + Vec2::new(24., 60.);
    let jitter = start + Vec2::new(1., 1.);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        vec![
            egui::Event::PointerMoved(start),
            pointer(start, true, egui::Modifiers::NONE),
        ],
    );
    let click = frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![
            egui::Event::PointerMoved(jitter),
            pointer(jitter, false, egui::Modifiers::NONE),
        ],
    );
    assert!(matches!(click.pick, Some(PickRequest::Point(_))));
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![pointer(start, true, egui::Modifiers::NONE)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![egui::Event::PointerMoved(start + Vec2::splat(30.))],
    );
    let hit = editor.snapshot(&mut assets)?.entities[0].id;
    editor.feedback(&feedback(&click, 2, vec![hit]));
    assert!(
        editor.selection.is_empty(),
        "a confirmed drag supersedes older click feedback"
    );
    let returned = frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![
            egui::Event::PointerMoved(start),
            pointer(start, false, egui::Modifiers::NONE),
        ],
    );
    assert!(returned.pick.is_none());
    assert!(editor.selection.is_empty());
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn gizmo_owns_its_press_but_cannot_capture_a_marquee_that_crosses_its_handles() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let snapshot = editor.snapshot(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    editor.selection.choose(Some(id), SelectMode::Replace);
    editor.gizmo_tool = GizmoTool::Scale;
    let original = editor.document.shared_source();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let view = frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let world = comet_scene::world_transforms(&view.snapshot)?[&id].transform_point3(Vec3::ZERO);
    let clip = view
        .camera
        .view_projection(view.viewport.aspect_ratio())
        .project_point3(world);
    let center = Pos2::new(
        view.viewport.center().x + clip.x * view.viewport.width() / 2.,
        view.viewport.center().y - clip.y * view.viewport.height() / 2.,
    );
    let end = center + Vec2::new(30., -30.);
    let pressed = frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        vec![
            egui::Event::PointerMoved(center),
            pointer(center, true, egui::Modifiers::NONE),
        ],
    );
    assert!(pressed.pick.is_none() && editor.drag_origin.is_some());
    let (_, output) = frame_with_output(
        &ctx,
        &mut editor,
        &mut assets,
        context(2),
        vec![egui::Event::PointerMoved(end)],
    );
    assert!(!output.shapes.iter().any(|shape| overlay(&shape.shape)));
    let released = frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![pointer(end, false, egui::Modifiers::NONE)],
    );
    assert!(released.pick.is_none());
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), original);
    let start = view.viewport.right_top() + Vec2::new(-25., 30.);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![
            egui::Event::PointerMoved(start),
            pointer(start, true, egui::Modifiers::NONE),
        ],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![
            egui::Event::PointerMoved(center),
            egui::Event::Key {
                key: egui::Key::E,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert!(editor.drag_origin.is_none());
    assert_eq!(editor.gizmo_tool, GizmoTool::Scale);
    let marquee = frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![pointer(center, false, egui::Modifiers::NONE)],
    );
    assert!(matches!(marquee.pick, Some(PickRequest::Rect { .. })));
    assert_eq!(editor.document.shared_source(), original);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn changing_gesture_context_cancels_and_swallows_release_without_resuming_held_input() -> Result<()>
{
    for reason in [
        "escape",
        "camera",
        "world",
        "selection",
        "document",
        "products",
        "tool",
        "space",
        "target",
        "viewport",
        "modal",
        "modal release",
        "inactive",
        "activity",
    ] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        let initial = frame(&ctx, &mut editor, &mut assets, 0, vec![]);
        let start = initial.viewport.right_top() + Vec2::new(-25., 30.);
        let end = start + Vec2::new(-90., 80.);
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            1,
            vec![
                egui::Event::PointerMoved(start),
                pointer(start, true, egui::Modifiers::NONE),
            ],
        );
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            2,
            vec![egui::Event::PointerMoved(end)],
        );
        let mut events = vec![];
        let mut changed = context(3);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        match reason {
            "escape" => events.push(key(egui::Key::Escape, egui::Modifiers::NONE)),
            "camera" => editor.camera.eye.x += 0.2,
            "world" => {
                editor.document =
                    comet_editor_core::Document::open_source(editor.document.source())?
            }
            "selection" => editor
                .selection
                .choose(Some(initial.snapshot.entities[0].id), SelectMode::Replace),
            "document" => editor
                .document
                .insert_entity(EntityData::new("external edit"), &mut assets)?,
            "products" => {
                fs::write(
                    editor.project_root().join("assets/new.rhai"),
                    "fn on_start(ctx) {}",
                )?;
                comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
                assets.refresh()?;
            }
            "tool" => editor.gizmo_tool = GizmoTool::Rotate,
            "space" => editor.transform_space = crate::transform::TransformSpace::Local,
            "target" => changed.target_generation = 2,
            "viewport" => {}
            "modal" => events.push(key(egui::Key::O, egui::Modifiers::COMMAND)),
            "modal release" => {
                events.push(pointer(end, false, egui::Modifiers::NONE));
                events.push(key(egui::Key::O, egui::Modifiers::COMMAND));
            }
            "inactive" => changed.active = false,
            "activity" => changed.activity_changed = true,
            _ => unreachable!(),
        }
        let expected = editor.document.shared_source();
        let history = editor.document.history_len();
        let invalidated = if reason == "viewport" {
            let mut request = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 720.))),
                    ..Default::default()
                },
                |ui| {
                    request = Some(editor.frame(ui, &mut assets, &changed).unwrap());
                },
            );
            output.textures_delta.clear();
            request.unwrap()
        } else {
            frame_with_context(&ctx, &mut editor, &mut assets, changed, events)
        };
        assert!(invalidated.pick.is_none(), "{reason}");
        if reason.starts_with("modal") {
            assert!(editor.documents.is_blocking());
            frame(
                &ctx,
                &mut editor,
                &mut assets,
                4,
                vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            );
        }
        assert!(
            frame(
                &ctx,
                &mut editor,
                &mut assets,
                5,
                vec![egui::Event::PointerMoved(end + Vec2::splat(10.))]
            )
            .pick
            .is_none(),
            "{reason}"
        );
        assert!(
            frame(
                &ctx,
                &mut editor,
                &mut assets,
                6,
                vec![pointer(end, false, egui::Modifiers::NONE)]
            )
            .pick
            .is_none(),
            "{reason}"
        );
        assert_eq!(editor.document.shared_source(), expected, "{reason}");
        assert_eq!(editor.document.history_len(), history, "{reason}");
        assert!(editor.drag_origin.is_none());
        // Only a fresh physical press can start another selection gesture.
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            7,
            vec![
                egui::Event::PointerMoved(start),
                pointer(start, true, egui::Modifiers::NONE),
            ],
        );
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            8,
            vec![egui::Event::PointerMoved(end)],
        );
        assert!(
            matches!(
                frame(
                    &ctx,
                    &mut editor,
                    &mut assets,
                    9,
                    vec![pointer(end, false, egui::Modifiers::NONE)]
                )
                .pick,
                Some(PickRequest::Rect { .. })
            ),
            "{reason}"
        );
    }
    Ok(())
}

#[test]
fn play_primary_drags_remain_game_input_and_never_request_a_rectangle() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let source = editor.document.shared_source();
    editor.start(&mut assets)?;
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let view = frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let start = view.viewport.center();
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        vec![
            egui::Event::PointerMoved(start),
            pointer(start, true, egui::Modifiers::NONE),
        ],
    );
    let drag = frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(start + Vec2::splat(50.))],
    );
    assert!(drag.pick.is_none());
    assert!(
        editor
            .play
            .as_ref()
            .unwrap()
            .input()
            .held
            .contains(comet_core::Action::Primary)
    );
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_marquee_overlay_draws_without_changing_authoring() -> Result<()> {
    use comet_render::{FramePacket, RenderWorker, UiFrame};
    let (_temp, mut editor, mut assets) = fixture()?;
    let source = editor.document.shared_source();
    let history = editor.document.history_len();
    let bytes = fs::read(&editor.source)?;
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    fs::create_dir_all(&directory)?;
    let capture = directory.join("editor-marquee.png");
    let mut worker = RenderWorker::spawn(None)?;
    let mut feedbacks = Vec::new();
    let mut viewport = Rect::NOTHING;
    for number in 0..8 {
        let start = viewport.left_top() + Vec2::splat(50.);
        let end = viewport.right_bottom() - Vec2::splat(50.);
        let events = match number {
            1 => vec![
                egui::Event::PointerMoved(start),
                pointer(start, true, egui::Modifiers::NONE),
            ],
            2 => vec![egui::Event::PointerMoved(end)],
            _ => vec![],
        };
        let mut request = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))),
                events,
                ..Default::default()
            },
            |ui| {
                request = Some(editor.frame(ui, &mut assets, &context(number)));
            },
        );
        let view = request.unwrap()?;
        viewport = view.viewport;
        assert!(view.pick.is_none() && editor.play.is_none());
        if number >= 2 {
            assert!(output.shapes.iter().any(|shape| overlay(&shape.shape)));
        }
        let ppp = output.pixels_per_point;
        let rect = viewport * ppp;
        let x = rect.min.x.clamp(0., 1280.) as u32;
        let y = rect.min.y.clamp(0., 800.) as u32;
        let ui = UiFrame {
            textures: std::mem::take(&mut output.textures_delta),
            primitives: ctx.tessellate(std::mem::take(&mut output.shapes), ppp),
            pixels_per_point: ppp,
        };
        worker.submit(FramePacket {
            number,
            token: ViewToken {
                world: view.world,
                view: 1,
                target_generation: 1,
            },
            size: [1280, 800],
            viewport: [
                x,
                y,
                (rect.width().max(0.) as u32).min(1280 - x),
                (rect.height().max(0.) as u32).min(800 - y),
            ],
            camera: view.camera,
            draws: comet_application::extract(&view.snapshot, &mut assets, &view.selected)?.into(),
            ui,
            pick: None,
            capture: (number == 7).then(|| capture.clone()),
            shadows: view.shadows,
        })?;
        let feedback = worker.wait(number)?;
        assert_eq!(feedback.number, number);
        assert!(feedback.draw_count > 0);
        assert_eq!(feedback.worker_thread, "comet-render");
        editor.feedback(&feedback);
        feedbacks.push(feedback);
    }
    worker.close()?;
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), history);
    assert_eq!(fs::read(&editor.source)?, bytes);
    assert!(!editor.document.is_dirty() && !editor.document.is_editing());
    assert!(editor.selection.is_empty() && editor.error.is_none());
    assert!(fs::read(&capture)?.starts_with(b"\x89PNG\r\n\x1a\n"));
    fs::write(
        directory.join("editor-marquee.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "headless":true,"submitted_frames":8,"frames":feedbacks,"application":editor.report(),
            "marquee":{"overlay_visible":true,"source_unchanged":true},
        }))?,
    )?;
    Ok(())
}
