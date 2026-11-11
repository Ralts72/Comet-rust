use super::*;
use comet_core::math::Quat;
use comet_scene::SceneSnapshot;

mod local_rotation;
mod local_translation;
mod rotation;
mod scale;
mod shortcuts;

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

fn project(view: &ViewRequest, point: Vec3) -> Pos2 {
    let ndc = view
        .camera
        .view_projection(view.viewport.aspect_ratio())
        .project_point3(point);
    Pos2::new(
        view.viewport.center().x + ndc.x * view.viewport.width() * 0.5,
        view.viewport.center().y - ndc.y * view.viewport.height() * 0.5,
    )
}

fn transformed_child(editor: &mut Editor) -> Result<comet_core::EntityId> {
    let mut parent = EntityData::new("parent");
    parent.transform.translation = [-0.25, 0.15, 0.3];
    parent.transform.rotation = Quat::from_rotation_y(0.6).to_array();
    parent.transform.scale = [2., 1.5, 0.75];
    let mut child = EntityData::new("child");
    child.parent = Some(parent.id);
    child.transform.translation = [0.17, 0.2, -0.1];
    child.transform.rotation = Quat::from_rotation_z(0.2).to_array();
    child.transform.scale = [0.5, 1., 2.];
    let id = child.id;
    editor.document = comet_editor_core::Document::open(SceneSnapshot {
        entities: vec![child, parent],
        ..Default::default()
    })?;
    editor.selection.replace([id]);
    Ok(id)
}

#[test]
fn snapped_world_axis_drag_preserves_parent_space_and_commits_once() -> Result<()> {
    for (enabled, step) in [(true, 0.5), (true, 0.25), (false, 0.5)] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let id = transformed_child(&mut editor)?;
        editor.translation_snap.enabled = enabled;
        editor.translation_snap.set_step(step)?;
        let before = editor.document.shared_source();
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        frame(&ctx, &mut editor, &mut assets, 0, vec![]);
        let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
        let origin =
            comet_scene::world_transforms(&view.snapshot)?[&id].transform_point3(Vec3::ZERO);
        let start = project(&view, origin + Vec3::X * 0.7);
        let axis = project(&view, origin + Vec3::X * 0.9) - project(&view, origin);
        let middle = start + axis * (0.63 / 0.9);
        let end = start + axis * (1.13 / 0.9);
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
            vec![egui::Event::PointerMoved(middle)],
        );
        assert!(
            editor.document.is_editing(),
            "Gizmo must own an active preview"
        );
        assert_eq!(editor.document.history_len(), 0);
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            4,
            vec![egui::Event::PointerMoved(end)],
        );
        let moved = editor.document.shared_source();
        assert_ne!(before, moved);
        let position =
            comet_scene::world_transforms(&moved.base)?[&id].transform_point3(Vec3::ZERO);
        let expected_x = if enabled {
            ((origin.x + 1.13) / step).round() * step
        } else {
            origin.x + 1.13
        };
        assert!((position.x - expected_x).abs() < 1e-4);
        assert!((position.y - origin.y).abs() < 1e-4);
        assert!((position.z - origin.z).abs() < 1e-4);
        let old_child = before
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap();
        let new_child = moved
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap();
        assert_eq!(old_child.transform.rotation, new_child.transform.rotation);
        assert_eq!(old_child.transform.scale, new_child.transform.scale);
        assert_ne!(
            old_child.transform.translation[2], new_child.transform.translation[2],
            "world X must convert through the rotated parent"
        );
        frame(&ctx, &mut editor, &mut assets, 5, vec![]);
        assert_eq!(editor.document.shared_source(), moved);
        frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
        assert_eq!(editor.document.history_len(), 1);
        assert!(!editor.document.is_editing());
        editor.document.undo()?;
        assert_eq!(
            editor.document.shared_source(),
            before,
            "history={} editing={} origin={:?} error={:?}",
            editor.document.history_len(),
            editor.document.is_editing(),
            editor.drag_origin,
            editor.error
        );
        editor.document.redo()?;
        assert_eq!(editor.document.shared_source(), moved);
    }
    Ok(())
}

#[test]
fn escape_cancels_snapped_drag_without_restarting_while_button_is_held() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = transformed_child(&mut editor)?;
    editor.translation_snap.enabled = true;
    let before = editor.document.shared_source();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let origin = comet_scene::world_transforms(&view.snapshot)?[&id].transform_point3(Vec3::ZERO);
    let start = project(&view, origin + Vec3::X * 0.7);
    let end = start + Vec2::new(60., 10.);
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
    assert_eq!(
        editor.document.shared_source(),
        before,
        "history={} editing={} origin={:?} error={:?}",
        editor.document.history_len(),
        editor.document.is_editing(),
        editor.drag_origin,
        editor.error
    );
    assert!(editor.drag_origin.is_none());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![egui::Event::PointerMoved(end + Vec2::new(40., 10.))],
    );
    frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_editing());
    Ok(())
}

#[test]
fn snap_settings_reject_invalid_steps_and_ui_toggle_does_not_edit_the_document() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let before = editor.document.shared_source();
    assert!(!editor.translation_snap.enabled);
    assert_eq!(editor.translation_snap.step(), 0.5);
    for invalid in [0., -1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(editor.translation_snap.set_step(invalid).is_err());
        assert_eq!(editor.translation_snap.step(), 0.5);
    }
    editor.translation_snap.set_step(0.25)?;
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
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
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    fn toggle(shape: &egui::epaint::Shape) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == "移动吸附" => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(toggle),
            _ => None,
        }
    }
    let toggle = output
        .shapes
        .iter()
        .find_map(|shape| toggle(&shape.shape))
        .expect("snap toggle must be visible");
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(toggle), button(toggle, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![button(toggle, false)],
    );
    assert!(editor.translation_snap.enabled);
    assert_eq!(editor.document.shared_source(), before);
    assert!(!editor.document.is_dirty());
    assert_eq!(editor.document.history_len(), 0);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![egui::Event::PointerMoved(toggle), button(toggle, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![button(toggle, false)],
    );
    assert!(!editor.translation_snap.enabled);
    editor.switch_document(editor.source.clone(), &mut assets)?;
    assert!(!editor.translation_snap.enabled);
    assert_eq!(editor.translation_snap.step(), 0.25);
    Ok(())
}

fn group_selection(editor: &mut Editor) -> Result<[comet_core::EntityId; 4]> {
    let mut parent = EntityData::new("unselected parent");
    parent.transform.rotation = Quat::from_rotation_y(0.5).to_array();
    parent.transform.scale = [2., 1.5, 0.75];
    let mut child = EntityData::new("selected child");
    child.parent = Some(parent.id);
    child.transform.translation = [-0.2, 0.1, 0.3];
    let mut root = EntityData::new("selected root");
    root.transform.translation = [0.1, 0.15, 0.2];
    root.transform.rotation = Quat::from_rotation_y(-0.3).to_array();
    let mut descendant = EntityData::new("selected descendant");
    descendant.parent = Some(root.id);
    descendant.transform.translation = [0.25, 0.2, -0.15];
    let ids = [parent.id, child.id, root.id, descendant.id];
    editor.document = comet_editor_core::Document::open(SceneSnapshot {
        entities: vec![child, descendant, root, parent],
        ..Default::default()
    })?;
    editor.selection.replace([ids[1], ids[2], ids[3]]);
    Ok(ids)
}

#[test]
fn group_drag_snaps_active_anchor_preserves_offsets_and_moves_selected_descendants_once()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = group_selection(&mut editor)?;
    editor.translation_snap.enabled = true;
    let before = editor.document.shared_source();
    let before_world = comet_scene::world_transforms(&before.base)?;
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let anchor = before_world[&ids[3]].transform_point3(Vec3::ZERO);
    let start = project(&view, anchor + Vec3::X * 0.7);
    let axis = project(&view, anchor + Vec3::X * 0.9) - project(&view, anchor);
    let middle = start + axis * (0.63 / 0.9);
    let end = start + axis * (1.13 / 0.9);
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
        vec![egui::Event::PointerMoved(middle)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![egui::Event::PointerMoved(end)],
    );
    let moved = editor.document.shared_source();
    let after_world = comet_scene::world_transforms(&moved.base)?;
    let expected_delta = Vec3::X * (((anchor.x + 1.13) / 0.5).round() * 0.5 - anchor.x);
    for id in &ids[1..] {
        let delta = after_world[id].transform_point3(Vec3::ZERO)
            - before_world[id].transform_point3(Vec3::ZERO);
        assert!(
            (delta - expected_delta).length() < 1e-4,
            "id={id} delta={delta:?} expected={expected_delta:?}"
        );
    }
    assert_eq!(before_world[&ids[0]], after_world[&ids[0]]);
    let original_child = before
        .base
        .entities
        .iter()
        .find(|e| e.id == ids[3])
        .unwrap();
    let moved_child = moved.base.entities.iter().find(|e| e.id == ids[3]).unwrap();
    assert_eq!(original_child, moved_child);
    assert_eq!(editor.document.history_len(), 0);
    frame(&ctx, &mut editor, &mut assets, 5, vec![button(end, false)]);
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), moved);
    Ok(())
}

#[test]
fn group_preview_updates_authored_and_direct_prefab_targets_atomically_and_can_cancel() -> Result<()>
{
    let (_temp, mut editor, mut assets) = fixture()?;
    let prefab_entity = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    let authored = EntityData::new("author target");
    let id = authored.id;
    editor.document.insert_entity(authored, &mut assets)?;
    editor.selection.replace([prefab_entity, id]);
    let before = editor.document.shared_source();
    let before_world = comet_scene::world_transforms(&before.expand(&mut assets)?)?;
    let history = editor.document.history_len();
    editor.begin_translation(&mut assets)?;
    editor.preview_translation(Vec3::new(0.5, 0.25, -0.75), &mut assets)?;
    let accepted = editor.document.shared_source();
    let after_world = comet_scene::world_transforms(&accepted.expand(&mut assets)?)?;
    for id in [id, prefab_entity] {
        let delta = after_world[&id].transform_point3(Vec3::ZERO)
            - before_world[&id].transform_point3(Vec3::ZERO);
        assert!((delta - Vec3::new(0.5, 0.25, -0.75)).length() < 1e-4);
    }
    assert!(
        editor
            .preview_translation(Vec3::splat(f32::NAN), &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.shared_source(), accepted);
    assert_eq!(editor.document.history_len(), history);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), history);
    Ok(())
}

#[test]
fn singular_parent_and_changed_selection_cannot_partially_move_a_group() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = group_selection(&mut editor)?;
    let mut source = editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == ids[0])
        .unwrap()
        .transform
        .scale[0] = 0.;
    editor.document = comet_editor_core::Document::open_source(source)?;
    let before = editor.document.shared_source();
    assert!(editor.begin_translation(&mut assets).is_err());
    assert_eq!(editor.document.shared_source(), before);
    assert!(!editor.document.is_editing());
    assert!(editor.drag_origin.is_none());
    group_selection(&mut editor)?;
    editor.begin_translation(&mut assets)?;
    let before = editor.document.shared_source();
    assert!(
        editor
            .preview_translation(Vec3::splat(f32::MAX), &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.shared_source(), before);
    editor.selection.clear();
    assert!(editor.preview_translation(Vec3::X, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), before);
    editor.cancel_transform()?;
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn escape_cancels_the_entire_group_and_does_not_resume_the_held_drag() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = group_selection(&mut editor)?;
    editor.translation_snap.enabled = true;
    let before = editor.document.shared_source();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let anchor =
        comet_scene::world_transforms(&view.snapshot)?[&ids[3]].transform_point3(Vec3::ZERO);
    let start = project(&view, anchor + Vec3::X * 0.7);
    let end = start + Vec2::new(70., 5.);
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
        vec![egui::Event::PointerMoved(end + Vec2::new(30., 0.))],
    );
    frame(&ctx, &mut editor, &mut assets, 6, vec![button(end, false)]);
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn a_nested_prefab_target_rejects_the_whole_group_before_opening_a_preview() -> Result<()> {
    use comet_core::InstanceId;
    use comet_scene::{ContentSource, PrefabInstance, PrefabResolver};
    let (temp, mut editor, mut assets) = fixture()?;
    let choice = editor
        .sources
        .of_kind(comet_asset_data::product::AssetKind::Prefab)
        .find(|choice| choice.source == std::path::Path::new("showcase.gltf"))
        .unwrap()
        .clone();
    let prefab = assets.prefab(choice.id)?;
    let leaf = prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    let anchor = EntityData::new("nested parent");
    let inner = InstanceId::new();
    let nested = ContentSource {
        base: SceneSnapshot {
            entities: vec![anchor.clone()],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: inner,
            prefab: choice.id,
            parent: Some(anchor.id),
            overrides: vec![],
        }],
        ..Default::default()
    };
    let project = temp.path().join("project");
    let path = project.join("assets/nested.prefab");
    fs::write(&path, nested.to_json()?)?;
    comet_asset_pipeline::prepare(&project, &editor.products)?;
    assets = comet_asset::AssetStore::open(&editor.products)?;
    let outer = InstanceId::new();
    let authored = EntityData::new("plain author target");
    let authored_id = authored.id;
    editor.document = comet_editor_core::Document::open_source(ContentSource {
        base: SceneSnapshot {
            entities: vec![authored],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: outer,
            prefab: comet_asset_pipeline::SourceMeta::load(&path)?.id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    let nested_id = leaf.in_instance(inner.in_instance(outer));
    assert!(
        editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .any(|entity| entity.id == nested_id)
    );
    editor.selection.replace([authored_id, nested_id]);
    let before = editor.document.shared_source();
    assert!(editor.begin_translation(&mut assets).is_err());
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_editing());
    assert!(editor.drag_origin.is_none());
    Ok(())
}

#[test]
fn document_modals_block_starting_and_changing_a_group_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    group_selection(&mut editor)?;
    let before = editor.document.shared_source();
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::NewScene);
    assert!(editor.begin_translation(&mut assets).is_err());
    assert!(!editor.document.is_editing());
    assert!(editor.drag_origin.is_none());
    assert_eq!(editor.document.shared_source(), before);
    editor.documents = Default::default();
    editor.begin_translation(&mut assets)?;
    editor.preview_translation(Vec3::X, &mut assets)?;
    let accepted = editor.document.shared_source();
    editor.documents.pending = true;
    assert!(editor.preview_translation(Vec3::Y, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn play_rejects_group_moves_but_single_non_physics_moves_can_cancel_without_touching_authoring()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = group_selection(&mut editor)?;
    let authoring = editor.document.shared_source();
    editor.start(&mut assets)?;
    editor.selection.replace([ids[1], ids[2]]);
    let before = editor.play.as_mut().unwrap().snapshot();
    assert!(editor.begin_translation(&mut assets).is_err());
    assert_eq!(editor.play.as_mut().unwrap().snapshot(), before);
    editor.selection.replace([ids[1]]);
    editor.begin_translation(&mut assets)?;
    editor.preview_translation(Vec3::X, &mut assets)?;
    assert_ne!(editor.play.as_mut().unwrap().snapshot(), before);
    assert_eq!(editor.document.shared_source(), authoring);
    editor.cancel_transform()?;
    assert_eq!(editor.play.as_mut().unwrap().snapshot(), before);
    assert_eq!(editor.document.shared_source(), authoring);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}
