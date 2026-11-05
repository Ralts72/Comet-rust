use super::*;
use crate::transform::{GizmoTool, TransformSpace};
use std::sync::Arc;

fn setup(editor: &mut Editor, rotation: Quat, group: bool) -> Result<comet_core::EntityId> {
    let mut primary = EntityData::new("local move pivot");
    primary.transform.translation = [0.17, 0.21, -0.13];
    primary.transform.rotation = rotation.to_array();
    let id = primary.id;
    let mut member = EntityData::new("local move member");
    member.transform.translation = [1.37, 0.21, -0.13];
    let member_id = member.id;
    editor.document = comet_editor_core::Document::open(SceneSnapshot {
        entities: if group {
            vec![primary, member]
        } else {
            vec![primary]
        },
        ..Default::default()
    })?;
    editor
        .selection
        .replace(if group { vec![member_id, id] } else { vec![id] });
    editor.gizmo_tool = GizmoTool::Translate;
    editor.transform_space = TransformSpace::Local;
    editor.camera.eye = Vec3::new(5., 3.5, 7.);
    editor.camera.target = Vec3::ZERO;
    Ok(id)
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    ctx
}
fn position(editor: &Editor, id: comet_core::EntityId) -> Vec3 {
    Vec3::from_array(
        editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .transform
            .translation,
    )
}
fn points(view: &ViewRequest, origin: Vec3, axis: Vec3, amount: f32) -> (Pos2, Pos2) {
    let start = project(view, origin + axis * 0.7);
    let pixels = project(view, origin + axis * 0.9) - project(view, origin);
    (start, start + pixels * (amount / 0.9))
}
fn start(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    id: comet_core::EntityId,
    axis: Vec3,
    amount: f32,
) -> (ViewRequest, Pos2, Pos2) {
    frame(ctx, editor, assets, 0, vec![]);
    let view = frame(ctx, editor, assets, 1, vec![]);
    let (start, end) = points(&view, position(editor, id), axis, amount);
    frame(
        ctx,
        editor,
        assets,
        2,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    frame(ctx, editor, assets, 3, vec![egui::Event::PointerMoved(end)]);
    assert!(editor.document.is_editing() && editor.drag_origin.is_some());
    (view, start, end)
}

#[test]
fn local_x_moves_the_entire_group_along_world_y_or_z_from_one_baseline() -> Result<()> {
    for (rotation, axis) in [
        (Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::Y),
        (Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2), Vec3::Z),
    ] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let id = setup(&mut editor, rotation, true)?;
        let before = editor.document.shared_source();
        let ctx = context();
        let (_, _, end) = start(&ctx, &mut editor, &mut assets, id, axis, 0.8);
        let accepted = editor.document.shared_source();
        for original in &before.base.entities {
            let moved = position(&editor, original.id);
            assert!(
                (moved - Vec3::from_array(original.transform.translation) - axis * 0.8).length()
                    < 1e-4
            );
        }
        frame(&ctx, &mut editor, &mut assets, 4, vec![button(end, false)]);
        assert_eq!(editor.document.history_len(), 1);
        editor.document.undo()?;
        assert_eq!(editor.document.shared_source(), before);
        editor.document.redo()?;
        assert_eq!(editor.document.shared_source(), accepted);
    }
    Ok(())
}

#[test]
fn world_snap_uses_the_absolute_grid_while_local_snap_uses_the_drag_origin() -> Result<()> {
    for (space, expected_y) in [(TransformSpace::World, 0.5), (TransformSpace::Local, 0.71)] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let id = setup(
            &mut editor,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            false,
        )?;
        editor.transform_space = space;
        editor.translation_snap.enabled = true;
        editor.translation_snap.set_step(0.5)?;
        let ctx = context();
        let (_, _, end) = start(&ctx, &mut editor, &mut assets, id, Vec3::Y, 0.3);
        assert!((position(&editor, id).y - expected_y).abs() < 1e-5);
        frame(&ctx, &mut editor, &mut assets, 4, vec![button(end, false)]);
        assert_eq!(editor.document.history_len(), 1);
    }
    Ok(())
}

#[test]
fn a_click_or_pointer_jitter_does_not_snap_an_off_grid_object() -> Result<()> {
    for space in [TransformSpace::World, TransformSpace::Local] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let id = setup(&mut editor, Quat::IDENTITY, false)?;
        editor.transform_space = space;
        editor.translation_snap.enabled = true;
        let before = editor.document.shared_source();
        let ctx = context();
        frame(&ctx, &mut editor, &mut assets, 0, vec![]);
        let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
        let (start, _) = points(&view, position(&editor, id), Vec3::X, 0.);
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
            vec![egui::Event::PointerMoved(start + Vec2::new(1., 1.))],
        );
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            4,
            vec![button(start + Vec2::new(1., 1.), false)],
        );
        assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
        assert_eq!(editor.document.history_len(), 0);
        assert!(!editor.document.is_editing() && editor.drag_origin.is_none());
    }
    Ok(())
}

#[test]
fn repeated_local_input_is_cached_and_returning_to_zero_restores_the_exact_source() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = setup(
        &mut editor,
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        false,
    )?;
    let before = editor.document.shared_source();
    let ctx = context();
    let (_, start, _) = start(&ctx, &mut editor, &mut assets, id, Vec3::Y, 0.8);
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![egui::Event::PointerMoved(start)],
    );
    assert_eq!(editor.document.shared_source(), before);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![button(start, false)],
    );
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn changing_move_space_or_tool_cancels_without_restarting_a_held_pointer() -> Result<()> {
    for space_change in [true, false] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let id = setup(
            &mut editor,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            false,
        )?;
        let before = editor.document.shared_source();
        let ctx = context();
        let (_, _, end) = start(&ctx, &mut editor, &mut assets, id, Vec3::Y, 0.8);
        if space_change {
            editor.transform_space = TransformSpace::World;
        } else {
            editor.gizmo_tool = GizmoTool::Rotate;
        }
        frame(&ctx, &mut editor, &mut assets, 4, vec![]);
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            5,
            vec![egui::Event::PointerMoved(end + Vec2::new(20., 10.))],
        );
        frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(editor.document.history_len(), 0);
        assert!(!editor.document.is_editing() && editor.drag_origin.is_none());
    }
    Ok(())
}

#[test]
fn moving_behind_the_camera_still_processes_release_and_commits_without_a_hanging_preview()
-> Result<()> {
    for space in [TransformSpace::World, TransformSpace::Local] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let rotation = if space == TransformSpace::Local {
            Quat::from_rotation_y(-std::f32::consts::FRAC_PI_4)
        } else {
            Quat::IDENTITY
        };
        let id = setup(&mut editor, rotation, false)?;
        editor.transform_space = space;
        let axis = rotation * Vec3::X;
        let ctx = context();
        let (view, _, end) = start(&ctx, &mut editor, &mut assets, id, axis, 30.);
        let moved = position(&editor, id);
        let projection = view.camera.view_projection(view.viewport.aspect_ratio());
        assert!(
            (projection * moved.extend(1.)).w < 0.,
            "the moved pivot must actually be behind the camera"
        );
        let accepted = editor.document.shared_source();
        frame(&ctx, &mut editor, &mut assets, 4, vec![]);
        frame(&ctx, &mut editor, &mut assets, 5, vec![button(end, false)]);
        assert_eq!(editor.document.shared_source(), accepted);
        assert_eq!(editor.document.history_len(), 1);
        assert!(!editor.document.is_editing() && editor.drag_origin.is_none());
        assert!(editor.error.is_none());
    }
    Ok(())
}
