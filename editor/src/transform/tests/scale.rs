use super::*;

#[test]
fn local_scale_uses_nonorthogonal_parent_axes_and_preserves_local_trs() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut parent = EntityData::new("rotated nonuniform parent");
    parent.transform.rotation = Quat::from_rotation_y(0.4).to_array();
    parent.transform.scale = [2., 1., 3.];
    let mut active = EntityData::new("reflected child");
    active.parent = Some(parent.id);
    active.transform.translation = [1., 2., 3.];
    active.transform.rotation = Quat::from_rotation_z(0.7).to_array();
    active.transform.scale = [-1., 2., 0.5];
    let id = active.id;
    let baseline = active.transform.clone();
    let basis = scale_basis(
        parent.transform.matrix(),
        Quat::from_array(baseline.rotation),
    )?;
    assert!(basis.x_axis.truncate().dot(basis.y_axis.truncate()).abs() > 0.1);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![parent, active],
        ..Default::default()
    })?;
    editor.selection.replace([id]);
    let before = editor.document.shared_source();
    let old_world = worlds(&editor, &mut assets)?[&id];
    editor.begin_scale(&mut assets)?;
    let pivot = editor.drag_origin.as_ref().unwrap().anchor;
    assert_eq!(
        editor.drag_origin.as_ref().unwrap().axes.unwrap()[0],
        basis.x_axis.truncate()
    );
    for factors in [Vec3::new(1.2, 0.8, 1.1), Vec3::new(1.5, 0.5, 2.)] {
        editor.preview_scale(factors, &mut assets)?;
    }
    let factors = Vec3::new(1.5, 0.5, 2.);
    let after = editor.document.shared_source();
    let value = &after
        .base
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .transform;
    assert_eq!(value.translation, baseline.translation);
    assert_eq!(value.rotation, baseline.rotation);
    assert_eq!(value.scale, [-1.5, 1., 1.]);
    let delta = Mat4::from_translation(pivot)
        * basis
        * Mat4::from_scale(factors)
        * basis.inverse()
        * Mat4::from_translation(-pivot);
    assert_matrix(worlds(&editor, &mut assets)?[&id], delta * old_world);
    let revision = editor.document.revision();
    editor.preview_scale(factors, &mut assets)?;
    assert_eq!(editor.document.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(
        &after,
        &editor.document.shared_source()
    ));
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), after);
    Ok(())
}

#[test]
fn uniform_group_scale_preserves_zero_axes_when_active_is_a_covered_descendant() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut root = EntityData::new("collapsed reflected root");
    root.transform.translation = [1., 2., 0.];
    root.transform.scale = [0., -2., 3.];
    let mut child = EntityData::new("active descendant");
    child.parent = Some(root.id);
    child.transform.translation = [1., 1., 0.];
    let original_child = child.clone();
    let mut other = EntityData::new("other root");
    other.transform.translation = [4., 0., 1.];
    other.transform.scale = [2., 3., 4.];
    let ids = [root.id, other.id, child.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![child, other, root],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    let old_world = worlds(&editor, &mut assets)?;
    editor.begin_scale(&mut assets)?;
    assert!(editor.drag_origin.as_ref().unwrap().axes.is_none());
    let pivot = editor.drag_origin.as_ref().unwrap().anchor;
    editor.preview_scale(Vec3::splat(2.), &mut assets)?;
    let accepted = editor.document.shared_source();
    let after_world = worlds(&editor, &mut assets)?;
    let delta = Mat4::from_translation(pivot)
        * Mat4::from_scale(Vec3::splat(2.))
        * Mat4::from_translation(-pivot);
    for id in ids {
        assert_matrix(after_world[&id], delta * old_world[&id]);
    }
    assert_eq!(
        accepted.base.entities.iter().find(|e| e.id == ids[2]),
        Some(&original_child)
    );
    assert_eq!(
        accepted
            .base
            .entities
            .iter()
            .find(|e| e.id == ids[0])
            .unwrap()
            .transform
            .scale,
        [0., -4., 6.]
    );
    assert!(
        editor
            .preview_scale(Vec3::new(2., 1., 1.), &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.shared_source(), accepted);
    editor.preview_scale(Vec3::ONE, &mut assets)?;
    assert_eq!(editor.document.shared_source(), before);
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 0);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn covered_active_child_does_not_use_the_single_root_shortcut() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut root = EntityData::new("selected ancestor");
    root.transform.scale = [2., 1., 3.];
    let mut child = EntityData::new("active child with different axes");
    child.parent = Some(root.id);
    child.transform.rotation = Quat::from_rotation_z(FRAC_PI_4).to_array();
    child.transform.translation = [1., 0., 0.];
    editor.selection.replace([root.id, child.id]);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![root, child],
        ..Default::default()
    })?;
    let before = editor.document.shared_source();
    editor.begin_scale(&mut assets)?;
    assert!(
        editor
            .preview_scale(Vec3::new(2., 1., 1.), &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.shared_source(), before);
    editor.preview_scale(Vec3::splat(2.), &mut assets)?;
    assert_ne!(editor.document.shared_source(), before);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn axis_group_scaling_uses_one_pivot_and_shear_failure_keeps_the_last_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut first = EntityData::new("first");
    first.transform.translation = [-2., 0., 0.];
    first.transform.rotation = Quat::from_rotation_z(0.3).to_array();
    first.transform.scale = [-2., 1., 3.];
    let mut active = EntityData::new("active");
    active.transform.translation = [1., 2., 0.];
    active.transform.rotation = first.transform.rotation;
    let ids = [first.id, active.id];
    editor.selection.replace(ids);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![first, active],
        ..Default::default()
    })?;
    let old_world = worlds(&editor, &mut assets)?;
    editor.begin_scale(&mut assets)?;
    let drag = editor.drag_origin.as_ref().unwrap();
    let pivot = drag.anchor;
    let (basis, inverse) = drag.scale_basis.unwrap();
    let factors = Vec3::new(2., 1., 1.);
    let delta = Mat4::from_translation(pivot)
        * basis
        * Mat4::from_scale(factors)
        * inverse
        * Mat4::from_translation(-pivot);
    editor.preview_scale(factors, &mut assets)?;
    let new_world = worlds(&editor, &mut assets)?;
    for id in ids {
        assert_matrix(new_world[&id], delta * old_world[&id]);
    }
    editor.cancel_transform()?;
    let mut source = editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|e| e.id == ids[0])
        .unwrap()
        .transform
        .rotation = Quat::from_rotation_z(0.8).to_array();
    editor.document = Document::open_source(source)?;
    editor.begin_scale(&mut assets)?;
    editor.preview_scale(Vec3::splat(1.5), &mut assets)?;
    let accepted = editor.document.shared_source();
    assert!(editor.preview_scale(factors, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    editor.cancel_transform()?;
    Ok(())
}

#[test]
fn scaling_rejects_invalid_factors_overflow_and_nonzero_axis_underflow_atomically() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    for (scale, factor) in [(1e-30, 1e-20), (f32::MAX, 2.)] {
        let mut root = EntityData::new("range boundary");
        root.transform.scale = [scale, 1., 1.];
        editor.selection.replace([root.id]);
        editor.document = Document::open(SceneSnapshot {
            entities: vec![root],
            ..Default::default()
        })?;
        let before = editor.document.shared_source();
        editor.begin_scale(&mut assets)?;
        assert!(
            editor
                .preview_scale(Vec3::splat(factor), &mut assets)
                .is_err()
        );
        assert_eq!(editor.document.shared_source(), before);
        assert_eq!(
            editor.drag_origin.as_ref().unwrap().scale_preview,
            Vec3::ONE
        );
        for invalid in [
            Vec3::ZERO,
            Vec3::new(-1., 1., 1.),
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
        ] {
            assert!(editor.preview_scale(invalid, &mut assets).is_err());
            assert_eq!(editor.document.shared_source(), before);
        }
        editor.cancel_transform()?;
    }
    Ok(())
}

#[test]
fn physics_scaling_keeps_positive_uniform_shape_rules() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    for character in [false, true] {
        let mut root = EntityData::new("round collider");
        root.physics = Some(PhysicsBody {
            motion: if character {
                BodyMotion::Kinematic
            } else {
                BodyMotion::Fixed
            },
            shape: if character {
                ColliderShape::Capsule {
                    half_height: 0.5,
                    radius: 0.3,
                }
            } else {
                ColliderShape::Sphere { radius: 0.5 }
            },
            ..Default::default()
        });
        root.character = character.then(Character::default);
        editor.selection.replace([root.id]);
        editor.document = Document::open(SceneSnapshot {
            entities: vec![root],
            ..Default::default()
        })?;
        editor.begin_scale(&mut assets)?;
        editor.preview_scale(Vec3::splat(2.), &mut assets)?;
        let accepted = editor.document.shared_source();
        assert!(
            editor
                .preview_scale(Vec3::new(2., 1., 1.), &mut assets)
                .is_err()
        );
        assert_eq!(editor.document.shared_source(), accepted);
        editor.preview_scale(Vec3::splat(2.), &mut assets)?;
        assert_eq!(editor.document.shared_source(), accepted);
        editor.cancel_transform()?;
    }
    Ok(())
}

#[test]
fn prefab_scaling_only_pins_scale_and_keeps_source_pose_inheritance() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let snapshot = editor.document.shared_source().expand(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    editor.selection.replace([id]);
    let before = editor.document.shared_source();
    editor.begin_scale(&mut assets)?;
    editor.preview_scale(Vec3::new(2., 3., 4.), &mut assets)?;
    let accepted = editor.document.shared_source();
    let overrides: Vec<_> = accepted
        .instances
        .iter()
        .flat_map(|instance| &instance.overrides)
        .collect();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].field, schema::SCALE);
    editor.preview_scale(Vec3::ONE, &mut assets)?;
    assert_eq!(editor.document.shared_source(), before);
    editor.preview_scale(Vec3::new(2., 3., 4.), &mut assets)?;
    editor.commit()?;
    let source = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    gltf["nodes"][1]["translation"] = serde_json::json!([2., 3., 4.]);
    let rotation = Quat::from_rotation_y(0.4).to_array();
    gltf["nodes"][1]["rotation"] = serde_json::json!(rotation);
    std::fs::write(source, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    let expanded = editor.document.shared_source().expand(&mut assets)?;
    let entity = expanded.entities.iter().find(|e| e.id == id).unwrap();
    assert_eq!(entity.transform.translation, [2., 3., 4.]);
    assert!(
        Quat::from_array(entity.transform.rotation).abs_diff_eq(Quat::from_array(rotation), 1e-6)
    );
    assert_eq!(entity.transform.scale, [2., 3., 4.]);
    Ok(())
}

#[test]
fn play_scale_is_single_non_physics_and_cancel_restores_runtime_only() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let visual = EntityData::new("visual");
    let mut body = EntityData::new("body");
    body.physics = Some(PhysicsBody::default());
    let ids = [visual.id, body.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![visual, body],
        ..Default::default()
    })?;
    let author = editor.document.shared_source();
    editor.start(&mut assets)?;
    editor.selection.replace(ids);
    assert!(editor.begin_scale(&mut assets).is_err());
    editor.selection.replace([ids[1]]);
    assert!(editor.begin_scale(&mut assets).is_err());
    editor.selection.replace([ids[0]]);
    let before = editor.play.as_mut().unwrap().snapshot();
    editor.begin_scale(&mut assets)?;
    editor.preview_scale(Vec3::new(2., 3., 4.), &mut assets)?;
    assert_ne!(editor.play.as_mut().unwrap().snapshot(), before);
    editor.cancel_transform()?;
    assert_eq!(editor.play.as_mut().unwrap().snapshot(), before);
    assert_eq!(editor.document.shared_source(), author);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn cached_scale_still_checks_selection_world_and_products() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let root = EntityData::new("root");
    let other = EntityData::new("other");
    let ids = [root.id, other.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![root, other],
        ..Default::default()
    })?;
    editor.selection.replace([ids[0]]);
    editor.begin_scale(&mut assets)?;
    editor.preview_scale(Vec3::splat(2.), &mut assets)?;
    let accepted = editor.document.shared_source();
    editor.selection.replace([ids[1]]);
    assert!(editor.preview_scale(Vec3::splat(2.), &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    editor.cancel_transform()?;
    editor.selection.replace([ids[0]]);
    editor.begin_scale(&mut assets)?;
    editor.preview_scale(Vec3::splat(2.), &mut assets)?;
    let accepted = editor.document.shared_source();
    let source = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    gltf["nodes"][1]["translation"][0] = serde_json::json!(-2.);
    std::fs::write(source, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    assert!(editor.preview_scale(Vec3::splat(2.), &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    editor.cancel_transform()?;
    editor.begin_scale(&mut assets)?;
    editor.document = Document::open(SceneSnapshot::default())?;
    assert!(editor.preview_scale(Vec3::ONE, &mut assets).is_err());
    Ok(())
}

#[test]
fn small_group_members_keep_real_relative_scale_and_position_changes() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut small = EntityData::new("small selected root");
    small.transform.scale = [2e-6; 3];
    small.transform.translation = [2e-6, 0., 0.];
    let id = small.id;
    let active = EntityData::new("active");
    editor.selection.replace([id, active.id]);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![small, active],
        ..Default::default()
    })?;
    editor.begin_scale(&mut assets)?;
    editor.preview_scale(Vec3::new(1.1, 1., 1.), &mut assets)?;
    let source = editor.document.shared_source();
    let transform = &source
        .base
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .transform;
    assert!((transform.scale[0] - 2.2e-6).abs() < 1e-12);
    assert_eq!([transform.scale[1], transform.scale[2]], [2e-6; 2]);
    assert!((transform.translation[0] - 2.2e-6).abs() < 1e-12);
    editor.cancel_transform()?;
    Ok(())
}

#[test]
fn scale_basis_uses_the_same_accepted_quaternion_matrix_as_scene_content() -> Result<()> {
    let parent = Mat4::from_scale_rotation_translation(
        Vec3::new(2., 1., 3.),
        Quat::from_rotation_y(0.4),
        Vec3::new(1., 2., 3.),
    );
    let rotation = Quat::from_axis_angle(Vec3::new(1., 2., 3.).normalize(), 0.9);
    let rotation = Quat::from_array(rotation.to_array().map(|component| component * 1.0002));
    let basis = scale_basis(parent, rotation)?;
    let source = parent * Mat4::from_quat(rotation);
    for (actual, original) in [basis.x_axis, basis.y_axis, basis.z_axis].into_iter().zip([
        source.x_axis,
        source.y_axis,
        source.z_axis,
    ]) {
        assert!(
            actual
                .truncate()
                .abs_diff_eq(original.truncate().normalize(), 1e-7)
        );
    }
    assert!(scale_basis(Mat4::from_scale(Vec3::ZERO), Quat::IDENTITY).is_err());
    assert!(scale_basis(Mat4::IDENTITY, Quat::from_array([f32::NAN; 4])).is_err());
    Ok(())
}

#[test]
fn axis_group_scale_does_not_invent_missing_axes_for_a_collapsed_other_member() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut collapsed = EntityData::new("collapsed other");
    collapsed.transform.scale = [0., 1., 1.];
    let mut active = EntityData::new("active");
    active.transform.translation = [2., 0., 0.];
    editor.selection.replace([collapsed.id, active.id]);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![collapsed, active],
        ..Default::default()
    })?;
    let before = editor.document.shared_source();
    editor.begin_scale(&mut assets)?;
    assert!(editor.drag_origin.as_ref().unwrap().axes.is_some());
    assert!(
        editor
            .preview_scale(Vec3::new(2., 1., 1.), &mut assets)
            .is_err()
    );
    assert_eq!(editor.document.shared_source(), before);
    editor.preview_scale(Vec3::splat(2.), &mut assets)?;
    assert_ne!(editor.document.shared_source(), before);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn an_active_collapsed_object_keeps_its_axes_and_can_scale_other_local_components() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut active = EntityData::new("active with a zero axis");
    active.transform.scale = [0., -2., 3.];
    active.transform.translation = [1., 2., 3.];
    active.transform.rotation = Quat::from_rotation_y(0.4).to_array();
    let baseline = active.transform.clone();
    editor.selection.replace([active.id]);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![active],
        ..Default::default()
    })?;
    let before = editor.document.shared_source();
    editor.begin_scale(&mut assets)?;
    assert!(editor.drag_origin.as_ref().unwrap().axes.is_some());
    editor.preview_scale(Vec3::new(2., 0.5, 3.), &mut assets)?;
    let accepted = editor.document.shared_source();
    assert_eq!(accepted.base.entities[0].transform.scale, [0., -1., 9.]);
    assert_eq!(
        accepted.base.entities[0].transform.translation,
        baseline.translation
    );
    assert_eq!(
        accepted.base.entities[0].transform.rotation,
        baseline.rotation
    );
    editor.preview_scale(Vec3::ONE, &mut assets)?;
    assert_eq!(editor.document.shared_source(), before);
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}
