use super::*;
use crate::transform::GizmoTool;

fn setup_rotation(editor: &mut Editor, group: bool) -> Result<comet_core::EntityId> {
    let active = EntityData::new("rotation pivot");
    let id = active.id;
    let mut other = EntityData::new("rotation member");
    other.transform.translation = [1.2, 0., 0.];
    let other_id = other.id;
    editor.document = comet_editor_core::Document::open(SceneSnapshot {
        entities: if group {
            vec![active, other]
        } else {
            vec![active]
        },
        ..Default::default()
    })?;
    editor
        .selection
        .replace(if group { vec![other_id, id] } else { vec![id] });
    editor.camera.eye = Vec3::new(0., 0., 6.);
    editor.camera.target = Vec3::ZERO;
    editor.gizmo_tool = GizmoTool::Rotate;
    Ok(id)
}

fn rotation_context() -> egui::Context {
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    ctx
}

fn ring_point(view: &ViewRequest, angle: f32) -> Pos2 {
    // With a face-on Z ring, the handle remains 66 logical points from its pivot.
    project(view, Vec3::ZERO) + Vec2::new(angle.cos(), -angle.sin()) * 66.
}

fn assert_z_rotation(editor: &Editor, id: comet_core::EntityId, angle: f32) {
    let source = editor.document.shared_source();
    let entity = source.base.entities.iter().find(|e| e.id == id).unwrap();
    let actual = Quat::from_array(entity.transform.rotation);
    let expected = Quat::from_rotation_z(angle);
    assert!(
        actual.dot(expected).abs() > 1. - 1e-5,
        "expected {angle} radians, got {actual:?}; error: {:?}",
        editor.error
    );
}

fn preview_rotation_drag(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
) -> (ViewRequest, Pos2) {
    frame(ctx, editor, assets, 0, vec![]);
    let view = frame(ctx, editor, assets, 1, vec![]);
    let start = ring_point(&view, 0.3);
    let end = ring_point(&view, 1.1);
    frame(
        ctx,
        editor,
        assets,
        2,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    frame(ctx, editor, assets, 3, vec![egui::Event::PointerMoved(end)]);
    assert!(editor.drag_origin.is_some(), "{:?}", editor.error);
    (view, end)
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

#[test]
fn world_ring_drag_rotates_the_group_from_its_start_and_commits_once() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let active = setup_rotation(&mut editor, true)?;
    let before = editor.document.shared_source();
    let ctx = rotation_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let start = ring_point(&view, 0.3);
    let middle = ring_point(&view, 0.5);
    let center = project(&view, Vec3::ZERO);
    let direction = Vec2::new(1_f32.cos(), -1_f32.sin());
    let edge = ((view.viewport.max.x - center.x) / direction.x)
        .min((view.viewport.min.y - center.y) / direction.y);
    let end = center + direction * (edge + 12.);
    assert!(!view.viewport.contains(end));
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    assert!(editor.drag_origin.is_some(), "{:?}", editor.error);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![egui::Event::PointerMoved(middle)],
    );
    assert_z_rotation(&editor, active, 0.2);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![egui::Event::PointerMoved(end)],
    );
    assert_z_rotation(&editor, active, 0.7);
    let rotated = editor.document.shared_source();
    let member = rotated
        .base
        .entities
        .iter()
        .find(|e| e.id != active)
        .unwrap();
    let expected = Quat::from_rotation_z(0.7) * Vec3::new(1.2, 0., 0.);
    assert!((Vec3::from_array(member.transform.translation) - expected).length() < 1e-4);
    assert_eq!(editor.document.history_len(), 0);
    let revision = editor.document.revision();
    frame(&ctx, &mut editor, &mut assets, 5, vec![]);
    assert_eq!(editor.document.shared_source(), rotated);
    assert_eq!(editor.document.revision(), revision);
    let released = frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
    assert!(
        released.pick.is_none(),
        "a Gizmo release must not also pick"
    );
    assert_eq!(editor.document.history_len(), 1);
    assert!(editor.drag_origin.is_none());
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), rotated);
    Ok(())
}

#[test]
fn rotation_snap_uses_degrees_and_keeps_turns_across_the_pi_boundary() -> Result<()> {
    for (step, movements, expected) in [
        (15., vec![20_f32, 38.], 45_f32),
        (100., vec![170_f32, 190., 220.], 200_f32),
    ] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let active = setup_rotation(&mut editor, false)?;
        assert!(!editor.rotation_snap.enabled);
        assert_eq!(editor.rotation_snap.step(), 15.);
        editor.rotation_snap.enabled = true;
        editor.rotation_snap.set_step(step)?;
        let ctx = rotation_context();
        frame(&ctx, &mut editor, &mut assets, 0, vec![]);
        let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
        let start = ring_point(&view, 0.3);
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            2,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        for (index, angle) in movements.iter().enumerate() {
            let point = ring_point(&view, 0.3 + angle.to_radians());
            frame(
                &ctx,
                &mut editor,
                &mut assets,
                index as u64 + 3,
                vec![egui::Event::PointerMoved(point)],
            );
        }
        assert_z_rotation(&editor, active, expected.to_radians());
        let end = ring_point(&view, 0.3 + movements.last().unwrap().to_radians());
        frame(&ctx, &mut editor, &mut assets, 9, vec![button(end, false)]);
        assert_eq!(editor.document.history_len(), 1);
    }
    Ok(())
}

#[test]
fn escape_cancels_rotation_and_a_held_pointer_cannot_restart_it() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup_rotation(&mut editor, false)?;
    let before = editor.document.shared_source();
    let ctx = rotation_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let start = ring_point(&view, 0.3);
    let end = ring_point(&view, 1.1);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![egui::Event::PointerMoved(end)],
    );
    assert_ne!(editor.document.shared_source(), before);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert_eq!(editor.document.shared_source(), before);
    assert!(editor.drag_origin.is_none());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![egui::Event::PointerMoved(start)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![button(start, false)],
    );
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn changing_the_camera_cancels_rotation_without_publishing_or_resuming_the_drag() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup_rotation(&mut editor, false)?;
    let before = editor.document.shared_source();
    let ctx = rotation_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let start = ring_point(&view, 0.3);
    let end = ring_point(&view, 1.1);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![egui::Event::PointerMoved(end)],
    );
    assert_ne!(editor.document.shared_source(), before);
    editor.camera.eye.z = 8.;
    frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    assert_eq!(editor.document.shared_source(), before);
    assert!(editor.drag_origin.is_none());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![egui::Event::PointerMoved(start)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![button(start, false)],
    );
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn ring_interiors_edge_on_axes_and_clipped_pivots_leave_normal_clicks_available() -> Result<()> {
    for depth in [0., 5.99, 6.01] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let active = setup_rotation(&mut editor, false)?;
        let mut source = editor.document.source();
        source.base.entities[0].transform.translation[2] = depth;
        editor.document = comet_editor_core::Document::open_source(source)?;
        editor.selection.replace([active]);
        let before = editor.document.shared_source();
        let ctx = rotation_context();
        frame(&ctx, &mut editor, &mut assets, 0, vec![]);
        let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
        let point = view.viewport.center();
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            2,
            vec![egui::Event::PointerMoved(point), button(point, true)],
        );
        assert!(editor.drag_origin.is_none());
        let clicked = frame(
            &ctx,
            &mut editor,
            &mut assets,
            3,
            vec![button(point, false)],
        );
        assert!(clicked.pick.is_some(), "depth={depth}; {:?}", editor.error);
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(editor.document.history_len(), 0);
        assert!(editor.error.is_none());
    }
    Ok(())
}

#[test]
fn ring_size_stays_usable_at_different_distances_and_a_click_does_not_edit() -> Result<()> {
    for distance in [3., 30.] {
        let (_temp, mut editor, mut assets) = fixture()?;
        setup_rotation(&mut editor, false)?;
        editor.camera.eye.z = distance;
        let before = editor.document.shared_source();
        let revision = editor.document.revision();
        let ctx = rotation_context();
        frame(&ctx, &mut editor, &mut assets, 0, vec![]);
        let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
        let point = ring_point(&view, 0.3);
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            2,
            vec![egui::Event::PointerMoved(point), button(point, true)],
        );
        assert!(editor.drag_origin.is_some(), "distance={distance}");
        let released = frame(
            &ctx,
            &mut editor,
            &mut assets,
            3,
            vec![button(point, false)],
        );
        assert!(released.pick.is_none());
        assert!(editor.drag_origin.is_none());
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(editor.document.revision(), revision);
        assert_eq!(editor.document.history_len(), 0);
    }
    Ok(())
}

#[test]
fn selecting_the_rotation_tool_and_its_snap_controls_keeps_authoring_unchanged() -> Result<()> {
    fn label_position(shape: &egui::epaint::Shape, text: &str, view: Rect) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(label)
                if label.galley.job.text == text
                    && label.pos.x >= view.min.x
                    && label.pos.x < view.max.x
                    && label.pos.y < view.min.y =>
            {
                Some(label.pos + label.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| label_position(shape, text, view)),
            _ => None,
        }
    }
    let (_temp, mut editor, mut assets) = fixture()?;
    setup_rotation(&mut editor, false)?;
    editor.gizmo_tool = GizmoTool::Translate;
    let before = editor.document.shared_source();
    let ctx = rotation_context();
    let context = |number| FrameContext {
        number,
        dt: 1. / 60.,
        active: true,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    };
    frame_with_output(&ctx, &mut editor, &mut assets, context(0), vec![]);
    let (view, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    let tool = output
        .shapes
        .iter()
        .find_map(|shape| label_position(&shape.shape, "旋转 E", view.viewport))
        .expect("rotation tool must be in the viewport toolbar");
    for (number, pressed) in [(2, true), (3, false)] {
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            number,
            vec![egui::Event::PointerMoved(tool), button(tool, pressed)],
        );
    }
    assert_eq!(editor.gizmo_tool, GizmoTool::Rotate);
    let (view, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(4), vec![]);
    let snap = output
        .shapes
        .iter()
        .find_map(|shape| label_position(&shape.shape, "旋转吸附", view.viewport))
        .expect("rotation snap must replace the translation snap control");
    for (number, pressed) in [(5, true), (6, false)] {
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            number,
            vec![egui::Event::PointerMoved(snap), button(snap, pressed)],
        );
    }
    assert!(editor.rotation_snap.enabled);
    assert!(!editor.translation_snap.enabled);
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_editing());
    Ok(())
}

#[test]
fn rejected_rotation_start_preserves_an_existing_unrelated_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup_rotation(&mut editor, false)?;
    let mut parent = EntityData::new("singular parent");
    parent.transform.scale = [0., 1., 1.];
    let mut child = EntityData::new("selected child");
    child.parent = Some(parent.id);
    let parent_id = parent.id;
    let child_id = child.id;
    editor.document = comet_editor_core::Document::open(SceneSnapshot {
        entities: vec![parent, child],
        ..Default::default()
    })?;
    editor.selection.replace([child_id]);
    editor.document.begin()?;
    editor
        .document
        .preview_name(parent_id, "pending name".into())?;
    let preview = editor.document.shared_source();
    let ctx = rotation_context();
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let point = ring_point(&view, 0.3);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    assert!(editor.error.is_some());
    assert!(editor.drag_origin.is_none());
    assert!(editor.document.is_editing());
    assert_eq!(editor.document.shared_source(), preview);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn a_modal_freezes_rotation_then_cancels_it_when_drawing_resumes() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup_rotation(&mut editor, false)?;
    let before = editor.document.shared_source();
    let ctx = rotation_context();
    let (view, end) = preview_rotation_drag(&ctx, &mut editor, &mut assets);
    let preview = editor.document.shared_source();
    assert_ne!(before, preview);
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    assert_eq!(editor.document.shared_source(), preview);
    frame(&ctx, &mut editor, &mut assets, 5, vec![escape()]);
    assert!(!editor.documents.is_blocking());
    assert_eq!(editor.document.shared_source(), preview);
    let point = ring_point(&view, 1.4);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![egui::Event::PointerMoved(point)],
    );
    assert_eq!(editor.document.shared_source(), before);
    assert!(editor.drag_origin.is_none());
    frame(&ctx, &mut editor, &mut assets, 7, vec![button(end, false)]);
    let normal = view.viewport.center();
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        8,
        vec![egui::Event::PointerMoved(normal), button(normal, true)],
    );
    let clicked = frame(
        &ctx,
        &mut editor,
        &mut assets,
        9,
        vec![button(normal, false)],
    );
    assert!(
        clicked.pick.is_some(),
        "the retired gesture cannot block a new click"
    );
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn saving_ends_the_rotation_gesture_without_cancelling_a_later_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let active = setup_rotation(&mut editor, false)?;
    let ctx = rotation_context();
    let (_, end) = preview_rotation_drag(&ctx, &mut editor, &mut assets);
    let rotated = editor.document.shared_source();
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    assert!(editor.drag_origin.is_none());
    assert!(!editor.document.is_editing());
    assert!(!editor.document.is_dirty(), "{:?}", editor.error);
    assert_eq!(editor.document.shared_source(), rotated);
    editor.document.begin()?;
    editor
        .document
        .preview_name(active, "later preview".into())?;
    let later = editor.document.shared_source();
    frame(&ctx, &mut editor, &mut assets, 5, vec![]);
    frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
    assert_eq!(editor.document.shared_source(), later);
    assert!(editor.document.is_editing());
    assert_eq!(editor.document.history_len(), 1);
    Ok(())
}

#[test]
fn a_suspended_rotation_cannot_cancel_a_newly_accepted_world_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup_rotation(&mut editor, false)?;
    let ctx = rotation_context();
    preview_rotation_drag(&ctx, &mut editor, &mut assets);
    let previous_world = editor.world();
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    frame(&ctx, &mut editor, &mut assets, 5, vec![escape()]);
    editor.switch_document(editor.source.clone(), &mut assets)?;
    assert_ne!(editor.world(), previous_world);
    assert!(editor.drag_origin.is_none());
    let entity = EntityData::new("new world object");
    let active = entity.id;
    editor.document.insert_entity(entity, &mut assets)?;
    editor.selection.replace([active]);
    editor.document.begin()?;
    editor
        .document
        .preview_name(active, "new world preview".into())?;
    let new_preview = editor.document.shared_source();
    frame(&ctx, &mut editor, &mut assets, 6, vec![]);
    assert_eq!(editor.document.shared_source(), new_preview);
    assert!(editor.document.is_editing());
    assert_eq!(editor.document.history_len(), 1);
    Ok(())
}
