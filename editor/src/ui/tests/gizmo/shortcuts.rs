use super::*;
use crate::transform::GizmoTool;

fn keys(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn setup(editor: &mut Editor) -> Result<comet_core::EntityId> {
    let entity = EntityData::new("keyboard target");
    let id = entity.id;
    editor.document = comet_editor_core::Document::open(SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    })?;
    editor.selection.replace([id]);
    editor.camera.eye = Vec3::new(0., 0., 6.);
    editor.camera.target = Vec3::ZERO;
    Ok(id)
}

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

fn ui_context() -> egui::Context {
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    ctx
}

fn label(shape: &egui::epaint::Shape, name: &str, region: Rect) -> Option<Pos2> {
    match shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == name => {
            let point = text.pos + text.galley.rect.center().to_vec2();
            region.contains(point).then_some(point)
        }
        egui::epaint::Shape::Vec(shapes) => {
            shapes.iter().find_map(|shape| label(shape, name, region))
        }
        _ => None,
    }
}

#[test]
fn unmodified_tool_keys_work_with_viewport_focus_without_editing_the_document() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup(&mut editor)?;
    let before = editor.document.shared_source();
    let ctx = ui_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let (view, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    for name in ["移动 W", "旋转 E", "缩放 R"] {
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| label(&shape.shape, name, Rect::EVERYTHING).is_some())
        );
    }
    let point = view.viewport.right_top() + Vec2::new(-30., 70.);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![button(point, false)],
    );
    for (number, key, expected) in [
        (4, egui::Key::E, GizmoTool::Rotate),
        (5, egui::Key::R, GizmoTool::Scale),
        (6, egui::Key::W, GizmoTool::Translate),
    ] {
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            number,
            keys(key, egui::Modifiers::NONE),
        );
        assert_eq!(editor.gizmo_tool, expected);
    }
    let mut ordered = keys(egui::Key::R, egui::Modifiers::NONE);
    ordered.extend(keys(egui::Key::E, egui::Modifiers::NONE));
    frame(&ctx, &mut editor, &mut assets, 7, ordered);
    assert_eq!(editor.gizmo_tool, GizmoTool::Rotate);
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn modifiers_and_key_repeat_never_select_a_tool_even_if_modifiers_are_released_in_the_frame()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup(&mut editor)?;
    let before = editor.document.shared_source();
    let ctx = ui_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    for (number, modifiers) in [
        egui::Modifiers::SHIFT,
        egui::Modifiers::ALT,
        egui::Modifiers::CTRL,
        egui::Modifiers::COMMAND,
        egui::Modifiers::MAC_CMD,
    ]
    .into_iter()
    .enumerate()
    {
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            number as u64 + 1,
            keys(egui::Key::E, modifiers),
        );
        assert_eq!(editor.gizmo_tool, GizmoTool::Translate);
    }
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![egui::Event::Key {
            key: egui::Key::R,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        }],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        7,
        vec![egui::Event::Key {
            key: egui::Key::R,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(editor.gizmo_tool, GizmoTool::Translate);
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn typing_tool_letters_in_an_inspector_field_does_not_switch_tools() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = setup(&mut editor)?;
    editor.gizmo_tool = GizmoTool::Scale;
    let before = editor.document.shared_source();
    let ctx = ui_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let (view, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    let point = output
        .shapes
        .iter()
        .find_map(|shape| {
            label(
                &shape.shape,
                "keyboard target",
                Rect::from_min_max(view.viewport.right_top(), Pos2::new(1280., 800.)),
            )
        })
        .expect("selected object name field must be visible");
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![button(point, false)],
    );
    assert!(ctx.text_edit_focused());
    let mut events = keys(egui::Key::W, egui::Modifiers::NONE);
    events.extend(keys(egui::Key::E, egui::Modifiers::NONE));
    events.extend(keys(egui::Key::R, egui::Modifiers::NONE));
    events.push(egui::Event::Text("wer".into()));
    frame(&ctx, &mut editor, &mut assets, 4, events);
    assert_eq!(editor.gizmo_tool, GizmoTool::Scale);
    let source = editor.document.shared_source();
    assert!(
        source
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .name
            .contains("wer")
    );
    assert_eq!(editor.document.history_len(), 0);
    editor.cancel()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn modals_inactive_hosts_and_live_drags_block_tool_switching() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup(&mut editor)?;
    let before = editor.document.shared_source();
    let ctx = ui_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        keys(egui::Key::R, egui::Modifiers::NONE),
    );
    assert_eq!(editor.gizmo_tool, GizmoTool::Translate);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        keys(egui::Key::Escape, egui::Modifiers::NONE),
    );
    assert!(!editor.documents.is_blocking());
    let mut inactive = context(3);
    inactive.active = false;
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        inactive,
        keys(egui::Key::E, egui::Modifiers::NONE),
    );
    assert_eq!(editor.gizmo_tool, GizmoTool::Translate);
    assert_eq!(editor.document.shared_source(), before);
    editor.gizmo_tool = GizmoTool::Scale;
    let view = frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    let point = project(&view, Vec3::ZERO);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    let end = point + Vec2::new(30., -30.);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![egui::Event::PointerMoved(end)],
    );
    assert!(editor.drag_origin.is_some());
    let preview = editor.document.shared_source();
    let mut attempts = keys(egui::Key::W, egui::Modifiers::NONE);
    attempts.extend(keys(egui::Key::E, egui::Modifiers::NONE));
    frame(&ctx, &mut editor, &mut assets, 7, attempts);
    assert_eq!(editor.gizmo_tool, GizmoTool::Scale);
    assert_eq!(editor.document.shared_source(), preview);
    assert_eq!(editor.document.history_len(), 0);
    frame(&ctx, &mut editor, &mut assets, 8, vec![button(end, false)]);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        9,
        keys(egui::Key::W, egui::Modifiers::NONE),
    );
    assert_eq!(editor.gizmo_tool, GizmoTool::Translate);
    assert_eq!(editor.document.history_len(), 1);
    Ok(())
}

#[test]
fn play_keeps_game_keys_and_r_restart_while_leaving_the_edit_tool_unchanged() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = setup(&mut editor)?;
    editor.gizmo_tool = GizmoTool::Rotate;
    let authoring = editor.document.shared_source();
    editor.start(&mut assets)?;
    editor.play.as_mut().unwrap().set_paused(true);
    let baseline = editor.play.as_mut().unwrap().snapshot();
    editor.edit(
        id,
        comet_scene::schema::TRANSLATION,
        comet_scene::schema::FieldValue::Vec3([5., 2., 1.]),
        &mut assets,
    )?;
    let ctx = ui_context();
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(0), vec![]);
    for name in ["移动 W", "旋转 E", "缩放 R"] {
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| { label(&shape.shape, name, Rect::EVERYTHING).is_none() })
        );
    }
    for (number, key) in [(1, egui::Key::W), (2, egui::Key::E)] {
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            number,
            keys(key, egui::Modifiers::NONE),
        );
        assert_eq!(editor.gizmo_tool, GizmoTool::Rotate);
    }
    let world = editor.world();
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        keys(egui::Key::R, egui::Modifiers::NONE),
    );
    assert_ne!(editor.world(), world);
    assert_eq!(editor.play.as_mut().unwrap().snapshot(), baseline);
    assert_eq!(editor.gizmo_tool, GizmoTool::Rotate);
    assert_eq!(editor.document.shared_source(), authoring);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}
