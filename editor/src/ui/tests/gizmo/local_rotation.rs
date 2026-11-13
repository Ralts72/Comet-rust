use super::*;
use crate::transform::{GizmoTool, TransformSpace};
use std::sync::Arc;

fn setup(editor: &mut Editor, group: bool) -> Result<comet_core::EntityId> {
    let mut primary = EntityData::new("local rotation pivot");
    primary.transform.rotation = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2).to_array();
    let id = primary.id;
    let mut member = EntityData::new("local rotation member");
    member.transform.translation = [1.2, 0., 0.];
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
    editor.camera.eye = Vec3::new(0., 0., 6.);
    editor.camera.target = Vec3::ZERO;
    editor.gizmo_tool = GizmoTool::Rotate;
    editor.transform_space = TransformSpace::Local;
    Ok(id)
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    ctx
}
fn ring_point(view: &ViewRequest, angle: f32) -> Pos2 {
    // The primary's local X points along world Z, so this red ring faces the camera.
    project(view, Vec3::ZERO) + Vec2::new(angle.cos(), -angle.sin()) * 66.
}
fn assert_rotation(editor: &Editor, id: comet_core::EntityId, angle: f32) {
    let source = editor.document.shared_source();
    let primary = source
        .base
        .entities
        .iter()
        .find(|entity| entity.id == id)
        .unwrap();
    let expected =
        Quat::from_rotation_z(angle) * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
    assert!(
        Quat::from_array(primary.transform.rotation)
            .dot(expected)
            .abs()
            > 1. - 1e-5
    );
}
fn drag(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    angle: f32,
) -> (ViewRequest, Pos2) {
    frame(ctx, editor, assets, 0, vec![]);
    let view = frame(ctx, editor, assets, 1, vec![]);
    let start = ring_point(&view, 0.3);
    let end = ring_point(&view, 0.3 + angle);
    frame(
        ctx,
        editor,
        assets,
        2,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    frame(ctx, editor, assets, 3, vec![egui::Event::PointerMoved(end)]);
    assert!(editor.drag_origin.is_some() && editor.document.is_editing());
    (view, end)
}

#[test]
fn local_x_ring_rotates_the_group_in_world_z_and_commits_one_history_item() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = setup(&mut editor, true)?;
    let before = editor.document.shared_source();
    let ctx = context();
    let (_, end) = drag(&ctx, &mut editor, &mut assets, 0.7);
    assert_rotation(&editor, id, 0.7);
    let accepted = editor.document.shared_source();
    let member = accepted
        .base
        .entities
        .iter()
        .find(|entity| entity.id != id)
        .unwrap();
    let expected = Quat::from_rotation_z(0.7) * Vec3::new(1.2, 0., 0.);
    assert!((Vec3::from_array(member.transform.translation) - expected).length() < 1e-4);
    let revision = editor.document.revision();
    frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    frame(&ctx, &mut editor, &mut assets, 5, vec![button(end, false)]);
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), accepted);
    Ok(())
}

#[test]
fn local_ring_snap_unwraps_angles_and_repeated_input_keeps_the_accepted_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = setup(&mut editor, false)?;
    editor.rotation_snap.enabled = true;
    editor.rotation_snap.set_step(45.)?;
    let ctx = context();
    let (view, _) = drag(&ctx, &mut editor, &mut assets, 170_f32.to_radians());
    let mut end = ring_point(&view, 0.3);
    for (index, angle) in [190_f32, 220.].into_iter().enumerate() {
        end = ring_point(&view, 0.3 + angle.to_radians());
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            index as u64 + 4,
            vec![egui::Event::PointerMoved(end)],
        );
    }
    assert_rotation(&editor, id, 225_f32.to_radians());
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![egui::Event::PointerMoved(end)],
    );
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    frame(&ctx, &mut editor, &mut assets, 7, vec![button(end, false)]);
    assert_eq!(editor.document.history_len(), 1);
    Ok(())
}

#[test]
fn escape_cancels_local_rotation_and_holding_the_pointer_cannot_restart_it() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    setup(&mut editor, false)?;
    let before = editor.document.shared_source();
    let ctx = context();
    let (_, end) = drag(&ctx, &mut editor, &mut assets, 0.6);
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
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![egui::Event::PointerMoved(end + Vec2::new(20., 10.))],
    );
    frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
    assert_eq!(editor.document.shared_source(), before);
    assert!(!editor.document.is_editing() && editor.drag_origin.is_none());
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn changing_transform_space_or_tool_cancels_and_requires_a_fresh_pointer_press() -> Result<()> {
    for change in [0, 1, 2] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let id = setup(&mut editor, false)?;
        let before = editor.document.shared_source();
        let ctx = context();
        let (view, end) = drag(&ctx, &mut editor, &mut assets, 0.6);
        match change {
            0 => editor.transform_space = TransformSpace::World,
            1 => editor.gizmo_tool = GizmoTool::Translate,
            _ => editor.gizmo_tool = GizmoTool::Scale,
        }
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            4,
            vec![egui::Event::PointerMoved(end)],
        );
        assert_eq!(editor.document.shared_source(), before);
        assert!(!editor.document.is_editing() && editor.drag_origin.is_none());
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            5,
            vec![egui::Event::PointerMoved(ring_point(&view, 1.2))],
        );
        frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(editor.document.history_len(), 0);
        editor.gizmo_tool = GizmoTool::Rotate;
        let view = frame(&ctx, &mut editor, &mut assets, 7, vec![]);
        let start = ring_point(&view, 0.3);
        let end = ring_point(&view, 0.8);
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            8,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            9,
            vec![egui::Event::PointerMoved(end)],
        );
        frame(&ctx, &mut editor, &mut assets, 10, vec![button(end, false)]);
        assert_rotation(&editor, id, 0.5);
        assert_eq!(editor.document.history_len(), 1);
    }
    Ok(())
}

#[test]
fn move_and_rotation_share_space_controls_without_editing_scene_content() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    assert_eq!(editor.transform_space, TransformSpace::World);
    setup(&mut editor, false)?;
    editor.transform_space = TransformSpace::World;
    let before = editor.document.shared_source();
    let ctx = context();
    let input = |number| FrameContext {
        gamepad: Default::default(),
        number,
        dt: 1. / 60.,
        active: true,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    };
    frame_with_output(&ctx, &mut editor, &mut assets, input(0), vec![]);
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, input(1), vec![]);
    let local = label_position(&output, "局部").unwrap();
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(local), button(local, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![button(local, false)],
    );
    assert_eq!(editor.transform_space, TransformSpace::Local);
    assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), 0);
    editor.gizmo_tool = GizmoTool::Translate;
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, input(4), vec![]);
    assert!(label_position(&output, "局部").is_some());
    assert!(label_position(&output, "世界").is_some());
    assert_eq!(editor.transform_space, TransformSpace::Local);
    editor.gizmo_tool = GizmoTool::Scale;
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, input(5), vec![]);
    assert!(label_position(&output, "局部").is_none());
    assert!(label_position(&output, "世界").is_none());
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_local_rotation_draws_the_selected_object_and_keeps_authoring_unchanged() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let author_file = editor.source.clone();
    let author_bytes = std::fs::read(&author_file)?;
    let model = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find_map(|entity| entity.model.clone())
        .expect("the fixture provides a renderable model");
    let id = setup(&mut editor, false)?;
    let mut source = editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == id)
        .unwrap()
        .model = Some(model);
    editor.document = comet_editor_core::Document::open_source(source)?;
    let world = editor.document.world_id();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    std::fs::create_dir_all(&output)?;
    let report = output.join("editor-local-rotation.json");
    let capture = output.join("editor-local-rotation.png");
    let mut options = comet_application::HostOptions::new("local rotation test");
    options.headless = true;
    options.smoke = true;
    options.report = Some(report.clone());
    options.capture = Some(capture.clone());
    comet_application::run(options, assets, editor)?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(report)?)?;
    assert_eq!(report["submitted_frames"], 8);
    assert_eq!(report["headless"], true);
    assert_eq!(report["application"]["play_active"], false);
    assert_eq!(report["application"]["dirty"], false);
    assert_eq!(report["application"]["history"], 0);
    assert!(report["application"]["error"].is_null());
    assert_eq!(report["application"]["selected"], serde_json::json!([id]));
    let frames = report["frames"].as_array().unwrap();
    assert_eq!(frames.last().unwrap()["draw_count"], 1);
    assert!(frames.iter().all(|frame| {
        frame["token"]["world"] == world.to_string()
            && frame["backend"]
                == if cfg!(target_os = "macos") {
                    "Metal"
                } else {
                    "Vulkan"
                }
    }));
    assert!(std::fs::metadata(capture)?.len() > 8);
    assert_eq!(std::fs::read(author_file)?, author_bytes);
    Ok(())
}

fn label_position(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
    fn find(shape: &egui::epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
}
