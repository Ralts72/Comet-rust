use super::*;
use std::sync::Arc;

fn assert_axes(actual: [Vec3; 3], expected: [Vec3; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            actual.abs_diff_eq(expected, 2e-6),
            "{actual:?} != {expected:?}"
        );
        assert!((actual.length_squared() - 1.).abs() < 2e-6);
    }
    assert!(actual[0].dot(actual[1]).abs() < 2e-6);
    assert!(actual[1].dot(actual[2]).abs() < 2e-6);
    assert!(actual[2].dot(actual[0]).abs() < 2e-6);
    assert!(actual[0].cross(actual[1]).abs_diff_eq(actual[2], 2e-6));
}

#[test]
fn local_axes_follow_the_authored_rotation_chain_without_scale_or_reflection() -> Result<()> {
    let mut root = EntityData::new("root");
    let root_rotation = Quat::from_rotation_y(0.4);
    root.transform.rotation = root_rotation.to_array();
    root.transform.scale = [-2., 0.5, 3.];
    let mut parent = EntityData::new("parent");
    parent.parent = Some(root.id);
    let parent_rotation = Quat::from_rotation_x(-0.3);
    parent.transform.rotation = parent_rotation.to_array();
    parent.transform.scale = [0., 4., -1.];
    let mut active = EntityData::new("active");
    active.parent = Some(parent.id);
    let active_rotation = Quat::from_rotation_z(0.6);
    active.transform.rotation = active_rotation.to_array();
    active.transform.scale = [-1., 2., 0.2];
    let id = active.id;
    let expected = (root_rotation * parent_rotation * active_rotation).normalize();
    let expected = [expected * Vec3::X, expected * Vec3::Y, expected * Vec3::Z];
    let mut snapshot = SceneSnapshot {
        entities: vec![active, root, parent],
        ..Default::default()
    };
    assert_axes(rigid_axes(&snapshot, id, TransformSpace::Local)?, expected);
    assert_axes(
        rigid_axes(&snapshot, id, TransformSpace::World)?,
        [Vec3::X, Vec3::Y, Vec3::Z],
    );
    for entity in &mut snapshot.entities {
        entity.transform.scale = [1.; 3];
    }
    assert_axes(rigid_axes(&snapshot, id, TransformSpace::Local)?, expected);
    Ok(())
}

#[test]
fn a_primary_descendant_defines_one_rigid_group_rotation_and_keeps_its_local_pose() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    assert_eq!(editor.transform_space, TransformSpace::World);
    editor.transform_space = TransformSpace::Local;
    let mut root = EntityData::new("selected ancestor");
    root.transform.translation = [1., 0., -2.];
    root.transform.rotation = Quat::from_rotation_y(0.4).to_array();
    root.transform.scale = [-2., 1., 3.];
    let mut active = EntityData::new("primary descendant");
    active.parent = Some(root.id);
    active.transform.translation = [1., 2., 0.3];
    active.transform.rotation = Quat::from_rotation_z(0.6).to_array();
    active.transform.scale = [-1., 0.7, 1.2];
    let original_active = active.clone();
    let mut other = EntityData::new("other selected root");
    other.transform.translation = [-3., 1., 0.2];
    other.transform.rotation = Quat::from_rotation_x(-0.3).to_array();
    other.transform.scale = [2., -0.7, 1.];
    let ids = [root.id, other.id, active.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![active, other, root],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    let before_world = worlds(&editor, &mut assets)?;
    let orientation = (Quat::from_rotation_y(0.4) * Quat::from_rotation_z(0.6)).normalize();
    let expected_axes = [
        orientation * Vec3::X,
        orientation * Vec3::Y,
        orientation * Vec3::Z,
    ];
    let pivot = before_world[&ids[2]].transform_point3(Vec3::ZERO);
    editor.begin_rotation(&mut assets)?;
    assert_axes(
        editor.drag_origin.as_ref().unwrap().axes.unwrap(),
        expected_axes,
    );
    assert!(
        editor
            .drag_origin
            .as_ref()
            .unwrap()
            .anchor
            .abs_diff_eq(pivot, 1e-6)
    );
    for angle in [0.3, 0.7] {
        editor.preview_rotation(expected_axes[0], angle, &mut assets)?;
    }
    assert_axes(
        editor.drag_origin.as_ref().unwrap().axes.unwrap(),
        expected_axes,
    );
    let accepted = editor.document.shared_source();
    let after_world = worlds(&editor, &mut assets)?;
    let delta = Mat4::from_translation(pivot)
        * Mat4::from_quat(Quat::from_axis_angle(expected_axes[0], 0.7))
        * Mat4::from_translation(-pivot);
    for id in ids {
        assert_matrix(after_world[&id], delta * before_world[&id]);
    }
    assert_eq!(
        accepted.base.entities.iter().find(|e| e.id == ids[2]),
        Some(&original_active)
    );
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), accepted);
    Ok(())
}

#[test]
fn a_nonuniform_reflected_parent_accepts_representable_rotation_and_rejects_group_shear()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let mut parent = EntityData::new("unselected nonuniform parent");
    let parent_rotation = Quat::from_rotation_y(0.4);
    parent.transform.rotation = parent_rotation.to_array();
    parent.transform.scale = [-2., 1., 3.];
    let mut active = EntityData::new("primary child");
    active.parent = Some(parent.id);
    active.transform.translation = [1., 0.2, 0.3];
    active.transform.scale = [1., 0.5, -0.75];
    let mut other = EntityData::new("other root");
    other.transform.translation = [-3., 1., 0.2];
    let ids = [other.id, active.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![active, other, parent],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    let before_world = worlds(&editor, &mut assets)?;
    editor.begin_rotation(&mut assets)?;
    let axis = editor.drag_origin.as_ref().unwrap().axes.unwrap()[2];
    assert!(axis.abs_diff_eq(parent_rotation * Vec3::Z, 1e-6));
    let pivot = editor.drag_origin.as_ref().unwrap().anchor;
    editor.preview_rotation(axis, FRAC_PI_2, &mut assets)?;
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    let delta = Mat4::from_translation(pivot)
        * Mat4::from_quat(Quat::from_axis_angle(axis, FRAC_PI_2))
        * Mat4::from_translation(-pivot);
    let after_world = worlds(&editor, &mut assets)?;
    for id in ids {
        assert_matrix(after_world[&id], delta * before_world[&id]);
    }
    assert!(
        editor
            .preview_rotation(axis, FRAC_PI_4, &mut assets)
            .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn local_prefab_rotation_only_overrides_rotation_and_restores_zero_or_complete_turns() -> Result<()>
{
    let (temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let path = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    gltf["nodes"][0]["rotation"] = serde_json::json!(Quat::from_rotation_y(0.4).to_array());
    gltf["nodes"][1]["rotation"] = serde_json::json!(Quat::from_rotation_x(0.3).to_array());
    gltf["nodes"][1]["scale"] = serde_json::json!([-2., 0.4, 1.5]);
    std::fs::write(&path, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    let snapshot = editor.document.shared_source().expand(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    editor.selection.replace([id]);
    let before = editor.document.shared_source();
    for final_angle in [0., TAU, -2. * TAU] {
        editor.begin_rotation(&mut assets)?;
        let axis = editor.drag_origin.as_ref().unwrap().axes.unwrap()[1];
        editor.preview_rotation(axis, 0.3, &mut assets)?;
        let accepted = editor.document.shared_source();
        let overrides: Vec<_> = accepted
            .instances
            .iter()
            .flat_map(|instance| &instance.overrides)
            .collect();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].field, schema::ROTATION);
        let revision = editor.document.revision();
        editor.preview_rotation(axis, 0.3, &mut assets)?;
        assert_eq!(editor.document.revision(), revision);
        assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
        editor.preview_rotation(axis, final_angle, &mut assets)?;
        assert_eq!(editor.document.shared_source(), before);
        editor.commit()?;
        assert_eq!(editor.document.history_len(), 0);
        assert!(!editor.document.is_dirty());
    }
    editor.begin_rotation(&mut assets)?;
    let axis = editor.drag_origin.as_ref().unwrap().axes.unwrap()[1];
    editor.preview_rotation(axis, 0.3, &mut assets)?;
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    editor.transform_space = TransformSpace::World;
    assert!(editor.preview_rotation(axis, 0.3, &mut assets).is_err());
    assert_eq!(editor.document.revision(), revision);
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    editor.transform_space = TransformSpace::Local;
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    gltf["nodes"][1]["translation"] = serde_json::json!([3.2, 0.7, -0.2]);
    gltf["nodes"][1]["scale"] = serde_json::json!([-3., 0.7, 2.]);
    std::fs::write(&path, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    let expanded = editor.document.shared_source().expand(&mut assets)?;
    let inherited = &expanded
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .transform;
    assert_eq!(inherited.translation, [3.2, 0.7, -0.2]);
    assert_eq!(inherited.scale, [-3., 0.7, 2.]);
    Ok(())
}

#[test]
fn local_play_rotation_is_isolated_and_rejects_physics_or_changed_space() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let mut parent = EntityData::new("parent");
    parent.transform.rotation = Quat::from_rotation_y(0.2).to_array();
    parent.transform.scale = [2.; 3];
    let mut child = EntityData::new("child");
    child.parent = Some(parent.id);
    child.transform.rotation = Quat::from_rotation_z(0.4).to_array();
    child.transform.translation = [1., 0., 0.];
    let mut body = EntityData::new("physics owner");
    body.transform.translation = [0., 5., 0.];
    body.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    let ids = [child.id, body.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![parent, child, body],
        ..Default::default()
    })?;
    let author = editor.document.shared_source();
    editor.start(&mut assets)?;
    editor.selection.replace([ids[0]]);
    let baseline = editor.play.as_mut().unwrap().shared_snapshot();
    editor.begin_rotation(&mut assets)?;
    let axes = editor.drag_origin.as_ref().unwrap().axes.unwrap();
    let orientation = (Quat::from_rotation_y(0.2) * Quat::from_rotation_z(0.4)).normalize();
    assert_axes(
        axes,
        [
            orientation * Vec3::X,
            orientation * Vec3::Y,
            orientation * Vec3::Z,
        ],
    );
    editor.preview_rotation(axes[0], 0.3, &mut assets)?;
    let accepted = editor.play.as_mut().unwrap().shared_snapshot();
    assert_ne!(accepted, baseline);
    editor.preview_rotation(axes[0], 0.3, &mut assets)?;
    assert!(Arc::ptr_eq(
        &accepted,
        &editor.play.as_mut().unwrap().shared_snapshot()
    ));
    editor.transform_space = TransformSpace::World;
    assert!(editor.preview_rotation(axes[0], 0.3, &mut assets).is_err());
    assert!(Arc::ptr_eq(
        &accepted,
        &editor.play.as_mut().unwrap().shared_snapshot()
    ));
    editor.cancel_transform()?;
    assert_eq!(editor.play.as_mut().unwrap().shared_snapshot(), baseline);
    editor.transform_space = TransformSpace::Local;
    editor.selection.replace([ids[1]]);
    assert!(editor.begin_rotation(&mut assets).is_err());
    assert!(editor.drag_origin.is_none());
    editor.selection.replace(ids);
    assert!(editor.begin_rotation(&mut assets).is_err());
    assert_eq!(editor.document.shared_source(), author);
    assert_eq!(editor.document.history_len(), 0);
    editor.stop();
    assert_eq!(editor.document.shared_source(), author);
    Ok(())
}
