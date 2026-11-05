use super::*;
use crate::transform::{GizmoTool, scale_basis};

fn setup(
    editor: &mut Editor,
    entities: Vec<EntityData>,
    selected: Vec<comet_core::EntityId>,
) -> Result<()> {
    editor.document = comet_editor_core::Document::open(SceneSnapshot {
        entities,
        ..Default::default()
    })?;
    editor.selection.replace(selected);
    editor.camera.eye = Vec3::new(0., 0., 6.);
    editor.camera.target = Vec3::ZERO;
    editor.gizmo_tool = GizmoTool::Scale;
    Ok(())
}

fn warm(editor: &mut Editor, assets: &mut AssetStore) -> (egui::Context, ViewRequest) {
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, editor, assets, 0, vec![]);
    let view = frame(&ctx, editor, assets, 1, vec![]);
    (ctx, view)
}

fn pointer_frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    number: u64,
    point: Pos2,
    pressed: Option<bool>,
) -> ViewRequest {
    let mut events = vec![egui::Event::PointerMoved(point)];
    if let Some(pressed) = pressed {
        events.push(button(point, pressed));
    }
    frame(ctx, editor, assets, number, events)
}

fn uniform_point(center: Pos2, factor: f32) -> Pos2 {
    center + Vec2::new(1., -1.).normalized() * (80. * (factor - 1.))
}

fn entity_scale(editor: &Editor, id: comet_core::EntityId) -> Vec3 {
    Vec3::from_array(
        editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .transform
            .scale,
    )
}

#[test]
fn center_drag_scales_the_group_from_its_baseline_and_undoes_once() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut active = EntityData::new("pivot");
    active.transform.rotation = Quat::from_rotation_z(0.3).to_array();
    active.transform.scale = [-2., 3., 0.5];
    let id = active.id;
    let mut other = EntityData::new("member");
    other.transform.translation = [1.2, 0., 0.];
    let other_id = other.id;
    setup(&mut editor, vec![active, other], vec![other_id, id])?;
    let before = editor.document.shared_source();
    let (ctx, view) = warm(&mut editor, &mut assets);
    let center = project(&view, Vec3::ZERO);
    pointer_frame(&ctx, &mut editor, &mut assets, 2, center, Some(true));
    assert!(editor.drag_origin.is_some(), "{:?}", editor.error);
    pointer_frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        uniform_point(center, 1.4),
        None,
    );
    assert!((entity_scale(&editor, id) - Vec3::new(-2.8, 4.2, 0.7)).length() < 1e-4);
    let end = uniform_point(center, 1.8);
    pointer_frame(&ctx, &mut editor, &mut assets, 4, end, None);
    assert!((entity_scale(&editor, id) - Vec3::new(-3.6, 5.4, 0.9)).length() < 1e-4);
    let scaled = editor.document.shared_source();
    let other = scaled
        .base
        .entities
        .iter()
        .find(|e| e.id == other_id)
        .unwrap();
    assert!(
        (Vec3::from_array(other.transform.translation) - Vec3::new(2.16, 0., 0.)).length() < 1e-4
    );
    let revision = editor.document.revision();
    frame(&ctx, &mut editor, &mut assets, 5, vec![]);
    assert_eq!(editor.document.revision(), revision);
    assert_eq!(editor.document.history_len(), 0);
    let released = pointer_frame(&ctx, &mut editor, &mut assets, 6, end, Some(false));
    assert!(released.pick.is_none());
    assert!(editor.drag_origin.is_none());
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), scaled);
    Ok(())
}

#[test]
fn axis_drag_uses_the_fixed_local_basis_and_ignores_the_objects_negative_scale_sign() -> Result<()>
{
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut parent = EntityData::new("nonuniform parent");
    parent.transform.rotation = Quat::from_rotation_z(0.35).to_array();
    parent.transform.scale = [2., 0.7, 1.];
    let mut child = EntityData::new("negative-scale child");
    child.parent = Some(parent.id);
    child.transform.rotation = Quat::from_rotation_z(0.65).to_array();
    child.transform.scale = [-2., 3., 1.];
    let id = child.id;
    let basis = scale_basis(
        parent.transform.matrix(),
        Quat::from_array(child.transform.rotation),
    )?;
    let direction = Vec2::new(basis.x_axis.x, -basis.x_axis.y);
    let original_parent = parent.clone();
    setup(&mut editor, vec![parent, child], vec![id])?;
    let (ctx, view) = warm(&mut editor, &mut assets);
    let start = project(&view, Vec3::ZERO) + direction * 60.;
    let end = start + direction * 48.;
    pointer_frame(&ctx, &mut editor, &mut assets, 2, start, Some(true));
    assert!(editor.drag_origin.is_some(), "{:?}", editor.error);
    pointer_frame(&ctx, &mut editor, &mut assets, 3, end, None);
    assert!(
        (entity_scale(&editor, id) - Vec3::new(-3.2, 3., 1.)).length() < 1e-4,
        "{:?}",
        editor.error
    );
    pointer_frame(&ctx, &mut editor, &mut assets, 4, end, Some(false));
    assert_eq!(editor.document.history_len(), 1);
    assert_eq!(
        editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|e| e.id == original_parent.id)
            .unwrap(),
        &original_parent
    );
    Ok(())
}

#[test]
fn scale_snap_is_relative_to_one_never_crosses_zero_and_can_restore_the_exact_baseline()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let entity = EntityData::new("snapped object");
    let id = entity.id;
    setup(&mut editor, vec![entity], vec![id])?;
    assert_eq!(editor.scale_snap.step(), 0.1);
    assert!(!editor.scale_snap.enabled);
    editor.scale_snap.enabled = true;
    let before = editor.document.shared_source();
    let (ctx, view) = warm(&mut editor, &mut assets);
    let center = project(&view, Vec3::ZERO);
    pointer_frame(&ctx, &mut editor, &mut assets, 2, center, Some(true));
    pointer_frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        uniform_point(center, 1.36),
        None,
    );
    assert!((entity_scale(&editor, id) - Vec3::splat(1.4)).length() < 1e-5);
    let revision = editor.document.revision();
    pointer_frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        uniform_point(center, 1.38),
        None,
    );
    assert_eq!(editor.document.revision(), revision);
    pointer_frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        uniform_point(center, -1.),
        None,
    );
    assert!((entity_scale(&editor, id) - Vec3::splat(0.1)).length() < 1e-5);
    pointer_frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        uniform_point(center, 30000.),
        None,
    );
    let maximum = entity_scale(&editor, id);
    assert!(maximum.min_element() > 9999.8 && maximum.max_element() <= 10000.);
    pointer_frame(&ctx, &mut editor, &mut assets, 7, center, None);
    assert_eq!(editor.document.shared_source(), before);
    pointer_frame(&ctx, &mut editor, &mut assets, 8, center, Some(false));
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn a_singular_local_basis_keeps_uniform_scaling_available_and_moves_a_selected_subtree_once()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut parent = EntityData::new("zero-scale parent");
    parent.transform.scale = [0., 1., 1.];
    let parent_id = parent.id;
    let mut child = EntityData::new("active descendant");
    child.parent = Some(parent_id);
    child.transform.translation = [0.1, 0.2, 0.];
    let child_id = child.id;
    let original_child = child.clone();
    setup(&mut editor, vec![parent, child], vec![parent_id, child_id])?;
    let (ctx, view) = warm(&mut editor, &mut assets);
    let pivot = Vec3::new(0., 0.2, 0.);
    let center = project(&view, pivot);
    pointer_frame(&ctx, &mut editor, &mut assets, 2, center, Some(true));
    assert!(
        editor
            .drag_origin
            .as_ref()
            .is_some_and(|drag| drag.axes.is_none()),
        "{:?}",
        editor.error
    );
    let end = uniform_point(center, 1.5);
    pointer_frame(&ctx, &mut editor, &mut assets, 3, end, None);
    assert!((entity_scale(&editor, parent_id) - Vec3::new(0., 1.5, 1.5)).length() < 1e-5);
    let source = editor.document.shared_source();
    assert_eq!(
        source
            .base
            .entities
            .iter()
            .find(|e| e.id == child_id)
            .unwrap(),
        &original_child
    );
    let world = comet_scene::world_transforms(&source.base)?;
    assert!((world[&child_id].transform_point3(Vec3::ZERO) - pivot).length() < 1e-5);
    pointer_frame(&ctx, &mut editor, &mut assets, 4, end, Some(false));
    assert_eq!(editor.document.history_len(), 1);
    Ok(())
}

#[test]
fn scale_escape_and_camera_changes_cancel_without_resuming_or_blocking_the_next_pick() -> Result<()>
{
    for escape in [true, false] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let entity = EntityData::new("cancelled object");
        let id = entity.id;
        setup(&mut editor, vec![entity], vec![id])?;
        let before = editor.document.shared_source();
        let (ctx, view) = warm(&mut editor, &mut assets);
        let center = project(&view, Vec3::ZERO);
        pointer_frame(&ctx, &mut editor, &mut assets, 2, center, Some(true));
        let end = uniform_point(center, 1.5);
        pointer_frame(&ctx, &mut editor, &mut assets, 3, end, None);
        assert_ne!(editor.document.shared_source(), before);
        let events = if escape {
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        } else {
            editor.camera.eye.z = 8.;
            vec![]
        };
        frame(&ctx, &mut editor, &mut assets, 4, events);
        assert_eq!(editor.document.shared_source(), before);
        assert!(editor.drag_origin.is_none());
        pointer_frame(
            &ctx,
            &mut editor,
            &mut assets,
            5,
            uniform_point(center, 1.9),
            None,
        );
        pointer_frame(&ctx, &mut editor, &mut assets, 6, end, Some(false));
        let normal = center + Vec2::new(30., 30.);
        pointer_frame(&ctx, &mut editor, &mut assets, 7, normal, Some(true));
        let clicked = pointer_frame(&ctx, &mut editor, &mut assets, 8, normal, Some(false));
        assert!(clicked.pick.is_some());
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(editor.document.history_len(), 0);
    }
    Ok(())
}

#[test]
fn an_invalid_physics_axis_scale_cancels_the_entire_group() -> Result<()> {
    use comet_scene::{ColliderShape, PhysicsBody};
    let (_temp, mut editor, mut assets) = fixture()?;
    let active = EntityData::new("ordinary pivot");
    let id = active.id;
    let mut sphere = EntityData::new("sphere");
    sphere.transform.translation[0] = 1.2;
    sphere.physics = Some(PhysicsBody {
        shape: ColliderShape::Sphere { radius: 0.5 },
        ..Default::default()
    });
    let sphere_id = sphere.id;
    setup(&mut editor, vec![active, sphere], vec![sphere_id, id])?;
    let before = editor.document.shared_source();
    let (ctx, view) = warm(&mut editor, &mut assets);
    let start = project(&view, Vec3::ZERO) + Vec2::X * 60.;
    pointer_frame(&ctx, &mut editor, &mut assets, 2, start, Some(true));
    assert!(editor.drag_origin.is_some());
    pointer_frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        start + Vec2::X * 40.,
        None,
    );
    assert!(editor.error.is_some());
    assert!(editor.drag_origin.is_none());
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn empty_axis_interiors_and_clipped_pivots_leave_normal_clicks_available() -> Result<()> {
    for depth in [0., 5.99, 6.01] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let mut entity = EntityData::new("clipped object");
        entity.transform.translation[2] = depth;
        let id = entity.id;
        setup(&mut editor, vec![entity], vec![id])?;
        let before = editor.document.shared_source();
        let (ctx, view) = warm(&mut editor, &mut assets);
        let point = view.viewport.center()
            + if depth == 0. {
                Vec2::splat(30.)
            } else {
                Vec2::ZERO
            };
        pointer_frame(&ctx, &mut editor, &mut assets, 2, point, Some(true));
        assert!(editor.drag_origin.is_none());
        let clicked = pointer_frame(&ctx, &mut editor, &mut assets, 3, point, Some(false));
        assert!(clicked.pick.is_some());
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(editor.document.history_len(), 0);
    }
    Ok(())
}

#[test]
fn clicking_uniform_scale_without_movement_keeps_prefab_overrides_untouched() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let snapshot = editor.snapshot(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    editor.selection.replace([id]);
    editor.gizmo_tool = GizmoTool::Scale;
    let before = editor.document.shared_source();
    let (ctx, view) = warm(&mut editor, &mut assets);
    let pivot = comet_scene::world_transforms(&view.snapshot)?[&id].transform_point3(Vec3::ZERO);
    let center = project(&view, pivot);
    pointer_frame(&ctx, &mut editor, &mut assets, 2, center, Some(true));
    assert!(editor.drag_origin.is_some(), "{:?}", editor.error);
    let released = pointer_frame(&ctx, &mut editor, &mut assets, 3, center, Some(false));
    assert!(released.pick.is_none());
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}
