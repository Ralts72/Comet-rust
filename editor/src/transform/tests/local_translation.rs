use super::*;
use std::sync::Arc;

fn assert_moved(
    after: &BTreeMap<EntityId, Mat4>,
    before: &BTreeMap<EntityId, Mat4>,
    ids: &[EntityId],
    delta: Vec3,
) {
    for id in ids {
        assert_matrix(after[id], Mat4::from_translation(delta) * before[id]);
    }
}

#[test]
fn local_group_translation_uses_the_primary_descendant_axes_and_moves_covered_children_once()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let mut grandparent = EntityData::new("unselected affine parent");
    let grandparent_rotation = Quat::from_rotation_y(0.4);
    grandparent.transform.rotation = grandparent_rotation.to_array();
    grandparent.transform.scale = [-2., 1., 3.];
    grandparent.transform.translation = [0.5, -0.3, 0.7];
    let unselected = grandparent.clone();
    let mut root = EntityData::new("selected root");
    root.parent = Some(grandparent.id);
    let root_rotation = Quat::from_rotation_x(0.3);
    root.transform.rotation = root_rotation.to_array();
    root.transform.scale = [0.5, -2., 1.5];
    root.transform.translation = [1., 0., -0.2];
    let mut active = EntityData::new("primary descendant");
    active.parent = Some(root.id);
    let active_rotation = Quat::from_rotation_z(0.6);
    active.transform.rotation = active_rotation.to_array();
    active.transform.scale = [-1., 0.7, 1.2];
    active.transform.translation = [1., 2., 0.3];
    let original_active = active.clone();
    let mut other = EntityData::new("other selected root");
    other.transform.rotation = Quat::from_rotation_x(-0.3).to_array();
    other.transform.scale = [2., -0.7, 1.];
    other.transform.translation = [-3., 1., 0.2];
    let ids = [root.id, other.id, active.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![active, other, grandparent, root],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    let before_world = worlds(&editor, &mut assets)?;
    let orientation = (grandparent_rotation * root_rotation * active_rotation).normalize();
    let expected = [
        orientation * Vec3::X,
        orientation * Vec3::Y,
        orientation * Vec3::Z,
    ];
    editor.begin_translation(&mut assets)?;
    let axes = editor.drag_origin.as_ref().unwrap().axes.unwrap();
    for (actual, expected) in axes.into_iter().zip(expected) {
        assert!(actual.abs_diff_eq(expected, 2e-6));
        assert!((actual.length_squared() - 1.).abs() < 2e-6);
    }
    let delta = axes[0] * 1.4 - axes[2] * 0.3;
    for movement in [delta * 0.5, delta] {
        editor.preview_translation(movement, &mut assets)?;
    }
    let accepted = editor.document.shared_source();
    let after_world = worlds(&editor, &mut assets)?;
    assert_moved(&after_world, &before_world, &ids, delta);
    assert_eq!(
        accepted.base.entities.iter().find(|e| e.id == ids[2]),
        Some(&original_active)
    );
    assert_eq!(
        accepted
            .base
            .entities
            .iter()
            .find(|e| e.id == unselected.id),
        Some(&unselected)
    );
    for id in ids {
        let old = before.base.entities.iter().find(|e| e.id == id).unwrap();
        let new = accepted.base.entities.iter().find(|e| e.id == id).unwrap();
        assert_eq!(new.transform.rotation, old.transform.rotation);
        assert_eq!(new.transform.scale, old.transform.scale);
    }
    assert_eq!(editor.drag_origin.as_ref().unwrap().axes.unwrap(), axes);
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), accepted);
    Ok(())
}

#[test]
fn mixed_author_and_prefab_translation_only_pins_translation_and_zero_restores_declarations()
-> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let expanded = editor.document.shared_source().expand(&mut assets)?;
    let prefab = expanded
        .entities
        .iter()
        .find(|e| e.name == "Blue cube")
        .unwrap();
    let prefab_id = prefab.id;
    let expected_axis = Quat::from_array(prefab.transform.rotation).normalize() * Vec3::X;
    let mut authored = EntityData::new("authored root");
    authored.transform.rotation = Quat::from_rotation_x(0.3).to_array();
    authored.transform.scale = [-2., 0.7, 1.5];
    authored.transform.translation = [2., 1., -1.];
    let authored_id = authored.id;
    editor.document.insert_entity(authored, &mut assets)?;
    editor.selection.replace([authored_id, prefab_id]);
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
    let before_world = worlds(&editor, &mut assets)?;
    editor.begin_translation(&mut assets)?;
    let axis = editor.drag_origin.as_ref().unwrap().axes.unwrap()[0];
    assert!(axis.abs_diff_eq(expected_axis, 2e-6));
    assert!(!axis.abs_diff_eq(Vec3::X, 0.1));
    let delta = axis * 1.2;
    editor.preview_translation(delta, &mut assets)?;
    let accepted = editor.document.shared_source();
    let overrides: Vec<_> = accepted
        .instances
        .iter()
        .flat_map(|instance| &instance.overrides)
        .collect();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].field, schema::TRANSLATION);
    assert_moved(
        &worlds(&editor, &mut assets)?,
        &before_world,
        &[authored_id, prefab_id],
        delta,
    );
    let revision = editor.document.revision();
    editor.preview_translation(delta, &mut assets)?;
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    editor.preview_translation(Vec3::ZERO, &mut assets)?;
    assert_eq!(editor.document.shared_source(), before);
    editor.commit()?;
    assert_eq!(editor.document.history_len(), history);
    editor.begin_translation(&mut assets)?;
    editor.preview_translation(delta, &mut assets)?;
    editor.commit()?;
    assert_eq!(editor.document.history_len(), history + 1);
    let path = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    let inherited_rotation = Quat::from_rotation_y(0.8);
    gltf["nodes"][2]["rotation"] = serde_json::json!(inherited_rotation.to_array());
    gltf["nodes"][2]["scale"] = serde_json::json!([2., 3., 4.]);
    std::fs::write(&path, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    let updated = editor.document.shared_source().expand(&mut assets)?;
    let inherited = &updated
        .entities
        .iter()
        .find(|e| e.id == prefab_id)
        .unwrap()
        .transform;
    assert!(Vec3::from_array(inherited.scale).abs_diff_eq(Vec3::new(2., 3., 4.), 1e-6));
    assert!(
        Quat::from_array(inherited.rotation)
            .dot(inherited_rotation)
            .abs()
            > 1. - 1e-6
    );
    Ok(())
}

#[test]
fn invalid_world_deltas_and_changed_space_keep_the_last_complete_preview_and_can_cancel()
-> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let root = EntityData::new("ordinary selected root");
    let mut parent = EntityData::new("tiny unselected parent");
    parent.transform.rotation = Quat::from_rotation_y(0.3).to_array();
    parent.transform.scale = [1e-3, 1., 1.];
    let mut child = EntityData::new("selected primary child");
    child.parent = Some(parent.id);
    let ids = [root.id, child.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![root, parent, child],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    editor.begin_translation(&mut assets)?;
    let delta = editor.drag_origin.as_ref().unwrap().axes.unwrap()[0] * 0.8;
    editor.preview_translation(delta, &mut assets)?;
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    for invalid in [
        Vec3::splat(f32::NAN),
        Vec3::splat(f32::INFINITY),
        Vec3::X * f32::MAX,
    ] {
        assert!(editor.preview_translation(invalid, &mut assets).is_err());
        assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
        assert_eq!(editor.document.revision(), revision);
        assert_eq!(
            editor.drag_origin.as_ref().unwrap().translation_preview,
            delta
        );
    }
    editor.transform_space = TransformSpace::World;
    assert!(editor.preview_translation(delta, &mut assets).is_err());
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn coordinate_space_preference_does_not_invalidate_local_scaling() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let mut root = EntityData::new("scaled root");
    root.transform.rotation = Quat::from_rotation_z(0.4).to_array();
    root.transform.scale = [-2., 1., 3.];
    let id = root.id;
    editor.document = Document::open(SceneSnapshot {
        entities: vec![root],
        ..Default::default()
    })?;
    editor.selection.replace([id]);
    let before = editor.document.shared_source();
    editor.begin_scale(&mut assets)?;
    let axes = editor.drag_origin.as_ref().unwrap().axes;
    let factors = Vec3::new(1.3, 0.5, 1.1);
    editor.preview_scale(factors, &mut assets)?;
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    editor.transform_space = TransformSpace::World;
    editor.preview_scale(factors, &mut assets)?;
    assert!(Arc::ptr_eq(&accepted, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    editor.preview_scale(Vec3::new(1.5, 0.8, 1.2), &mut assets)?;
    assert_ne!(editor.document.shared_source(), accepted);
    assert_eq!(editor.drag_origin.as_ref().unwrap().axes, axes);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn local_play_translation_preserves_authoring_and_rejects_physics_or_multiple_targets() -> Result<()>
{
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.transform_space = TransformSpace::Local;
    let mut parent = EntityData::new("affine parent");
    parent.transform.rotation = Quat::from_rotation_y(0.3).to_array();
    parent.transform.scale = [-2., 1., 3.];
    let mut child = EntityData::new("non-physics child");
    child.parent = Some(parent.id);
    child.transform.rotation = Quat::from_rotation_z(0.4).to_array();
    child.transform.translation = [1., 0.2, 0.];
    let mut body = EntityData::new("physics body");
    body.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    body.transform.translation = [0., 5., 0.];
    let ids = [child.id, body.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![parent, child, body],
        ..Default::default()
    })?;
    let author = editor.document.shared_source();
    editor.start(&mut assets)?;
    editor.selection.replace([ids[0]]);
    let baseline = editor.play.as_mut().unwrap().shared_snapshot();
    let before_world = comet_scene::world_transforms(&baseline)?;
    editor.begin_translation(&mut assets)?;
    let axis = editor.drag_origin.as_ref().unwrap().axes.unwrap()[1];
    let expected = (Quat::from_rotation_y(0.3) * Quat::from_rotation_z(0.4)).normalize() * Vec3::Y;
    assert!(axis.abs_diff_eq(expected, 2e-6));
    let delta = axis * 1.4;
    editor.preview_translation(delta, &mut assets)?;
    let accepted = editor.play.as_mut().unwrap().shared_snapshot();
    assert_moved(
        &comet_scene::world_transforms(&accepted)?,
        &before_world,
        &[ids[0]],
        delta,
    );
    editor.preview_translation(delta, &mut assets)?;
    assert!(Arc::ptr_eq(
        &accepted,
        &editor.play.as_mut().unwrap().shared_snapshot()
    ));
    editor.transform_space = TransformSpace::World;
    assert!(editor.preview_translation(delta, &mut assets).is_err());
    assert!(Arc::ptr_eq(
        &accepted,
        &editor.play.as_mut().unwrap().shared_snapshot()
    ));
    editor.cancel_transform()?;
    assert_eq!(editor.play.as_mut().unwrap().shared_snapshot(), baseline);
    editor.transform_space = TransformSpace::Local;
    editor.selection.replace([ids[1]]);
    assert!(editor.begin_translation(&mut assets).is_err());
    assert!(editor.drag_origin.is_none());
    editor.selection.replace(ids);
    assert!(editor.begin_translation(&mut assets).is_err());
    assert!(Arc::ptr_eq(&author, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), 0);
    editor.stop();
    assert_eq!(editor.document.shared_source(), author);
    Ok(())
}
