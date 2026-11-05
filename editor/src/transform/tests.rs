mod local_rotation;
mod local_translation;
mod scale;
use super::*;
use crate::test_support::fixture;
use comet_editor_core::Document;
use comet_scene::{BodyMotion, Character, ColliderShape, EntityData, PhysicsBody, SceneSnapshot};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

fn worlds(editor: &Editor, assets: &mut AssetStore) -> Result<BTreeMap<EntityId, Mat4>> {
    comet_scene::world_transforms(&editor.document.shared_source().expand(assets)?)
}

fn assert_matrix(actual: Mat4, expected: Mat4) {
    assert!(
        actual.abs_diff_eq(expected, 3e-5),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn rotation_uses_world_pivot_baselines_and_moves_selected_descendants_once() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut parent = EntityData::new("unselected rotated parent");
    parent.transform.rotation = Quat::from_rotation_y(0.4).to_array();
    parent.transform.scale = [2.; 3];
    let mut child = EntityData::new("selected reflected child");
    child.parent = Some(parent.id);
    child.transform.translation = [1., 0.2, 0.3];
    child.transform.rotation = Quat::from_rotation_x(0.3).to_array();
    child.transform.scale = [-1., 2., 0.7];
    let mut root = EntityData::new("selected root");
    root.transform.translation = [-2., 0.3, 0.4];
    let mut descendant = EntityData::new("selected descendant");
    descendant.parent = Some(root.id);
    descendant.transform.translation = [0.2, 1., 0.3];
    let ids = [child.id, root.id, descendant.id];
    let original_descendant = descendant.clone();
    editor.document = Document::open(SceneSnapshot {
        entities: vec![child, parent, root, descendant],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    let before_world = worlds(&editor, &mut assets)?;
    editor.begin_rotation(&mut assets)?;
    let pivot = before_world[&ids[2]].transform_point3(Vec3::ZERO);
    assert_eq!(editor.drag_origin.as_ref().unwrap().anchor, pivot);
    for angle in [0.2, 0.5, 0.8] {
        editor.preview_rotation(Vec3::Z, angle, &mut assets)?;
    }
    let accepted = editor.document.shared_source();
    let after_world = worlds(&editor, &mut assets)?;
    let delta =
        Mat4::from_translation(pivot) * Mat4::from_rotation_z(0.8) * Mat4::from_translation(-pivot);
    for id in ids {
        assert_matrix(after_world[&id], delta * before_world[&id]);
    }
    assert_eq!(
        accepted.base.entities.iter().find(|e| e.id == ids[2]),
        Some(&original_descendant)
    );
    assert_eq!(editor.document.history_len(), 0);
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), accepted);
    Ok(())
}

#[test]
fn authored_and_direct_prefab_rotations_share_one_candidate_and_cancel() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let expanded = editor.document.shared_source().expand(&mut assets)?;
    let prefab = expanded
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    let mut authored = EntityData::new("authored");
    authored.transform.translation = [2., 1., -1.];
    let id = authored.id;
    editor.document.insert_entity(authored, &mut assets)?;
    editor.selection.replace([prefab, id]);
    let before = editor.document.shared_source();
    let before_world = worlds(&editor, &mut assets)?;
    let history = editor.document.history_len();
    editor.begin_rotation(&mut assets)?;
    editor.preview_rotation(Vec3::Y, FRAC_PI_2, &mut assets)?;
    let after_world = worlds(&editor, &mut assets)?;
    let pivot = before_world[&id].transform_point3(Vec3::ZERO);
    let delta = Mat4::from_translation(pivot)
        * Mat4::from_rotation_y(FRAC_PI_2)
        * Mat4::from_translation(-pivot);
    for id in [id, prefab] {
        assert_matrix(after_world[&id], delta * before_world[&id]);
    }
    assert_ne!(editor.document.shared_source().instances, before.instances);
    editor.preview_rotation(Vec3::Y, TAU, &mut assets)?;
    assert_eq!(editor.document.shared_source(), before);
    editor.preview_rotation(Vec3::Y, FRAC_PI_2, &mut assets)?;
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), history);
    assert!(editor.drag_origin.is_none());
    Ok(())
}

#[test]
fn zero_and_complete_turns_restore_exact_trs_and_create_no_history() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut parent = EntityData::new("parent");
    parent.transform.rotation = Quat::from_rotation_y(0.7).to_array();
    parent.transform.scale = [2.; 3];
    let mut child = EntityData::new("child");
    child.parent = Some(parent.id);
    child.transform.rotation = (-Quat::from_rotation_z(0.3)).to_array();
    child.transform.translation = [0.2, 0.4, 0.3];
    child.transform.scale = [-2., 0.4, 1.5];
    let id = child.id;
    editor.document = Document::open(SceneSnapshot {
        entities: vec![parent, child],
        ..Default::default()
    })?;
    editor.selection.replace([id]);
    let before = editor.document.shared_source();
    for angle in [
        0.,
        TAU,
        -TAU,
        TAU * 2.,
        TAU * 3.,
        -TAU * 3.,
        TAU * 5.,
        TAU * 10.,
    ] {
        editor.begin_rotation(&mut assets)?;
        editor.preview_rotation(Vec3::Y, 0.8, &mut assets)?;
        assert_ne!(editor.document.shared_source(), before);
        editor.preview_rotation(Vec3::Y, angle, &mut assets)?;
        assert_eq!(editor.document.shared_source(), before);
        editor.commit()?;
        editor.drag_origin = None;
        assert_eq!(editor.document.history_len(), 0);
        assert!(!editor.document.is_dirty());
    }
    Ok(())
}

#[test]
fn shear_or_invalid_axis_keeps_the_last_valid_group_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut parent = EntityData::new("nonuniform parent");
    parent.transform.scale = [2., 1., 3.];
    let mut child = EntityData::new("child");
    child.parent = Some(parent.id);
    let root = EntityData::new("other selected root");
    let ids = [child.id, root.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![parent, child, root],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    editor.begin_rotation(&mut assets)?;
    editor.preview_rotation(Vec3::Z, FRAC_PI_2, &mut assets)?;
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    for (axis, angle) in [
        (Vec3::Z, FRAC_PI_4),
        (Vec3::ZERO, 0.),
        (Vec3::X * 2., 0.2),
        (Vec3::splat(f32::NAN), 0.2),
        (Vec3::X, f32::INFINITY),
    ] {
        assert!(editor.preview_rotation(axis, angle, &mut assets).is_err());
        assert_eq!(editor.document.shared_source(), accepted);
        assert_eq!(editor.document.revision(), revision);
    }
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn character_tilt_and_singular_group_member_do_not_partially_publish() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut character = EntityData::new("character");
    character.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    character.character = Some(Character::default());
    let root = EntityData::new("other selected root");
    let ids = [character.id, root.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![character, root],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    editor.begin_rotation(&mut assets)?;
    editor.preview_rotation(Vec3::Y, 0.3, &mut assets)?;
    let accepted = editor.document.shared_source();
    assert!(editor.preview_rotation(Vec3::X, 0.3, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    editor.cancel_transform()?;
    let mut flat = EntityData::new("flat visual");
    flat.transform.scale = [0., 1., 1.];
    let other = EntityData::new("other");
    editor.selection.replace([flat.id, other.id]);
    editor.document = Document::open(SceneSnapshot {
        entities: vec![flat, other],
        ..Default::default()
    })?;
    let before = editor.document.shared_source();
    editor.begin_rotation(&mut assets)?;
    assert!(editor.preview_rotation(Vec3::X, 0.3, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), before);
    editor.cancel_transform()?;
    Ok(())
}

#[test]
fn play_rotation_is_single_non_physics_atomic_and_cancel_restores_all_fields() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut parent = EntityData::new("parent");
    parent.transform.rotation = Quat::from_rotation_y(0.3).to_array();
    let mut child = EntityData::new("child");
    child.parent = Some(parent.id);
    child.transform.translation = [1., 0., 0.];
    let ids = [parent.id, child.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![parent, child],
        ..Default::default()
    })?;
    let author = editor.document.shared_source();
    editor.start(&mut assets)?;
    editor.selection.replace(ids);
    assert!(editor.begin_rotation(&mut assets).is_err());
    editor.selection.replace([ids[1]]);
    let baseline = editor.play.as_mut().unwrap().snapshot();
    editor.begin_rotation(&mut assets)?;
    editor.preview_rotation(Vec3::Z, 0.5, &mut assets)?;
    assert_ne!(editor.play.as_mut().unwrap().snapshot(), baseline);
    editor.cancel_transform()?;
    assert_eq!(editor.play.as_mut().unwrap().snapshot(), baseline);
    assert_eq!(editor.document.shared_source(), author);
    editor.stop();
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn selection_and_product_changes_reject_rotation_without_replacing_accepted_preview() -> Result<()>
{
    let (temp, mut editor, mut assets) = fixture()?;
    let root = EntityData::new("root");
    let other = EntityData::new("other");
    let ids = [root.id, other.id];
    editor.document = Document::open(SceneSnapshot {
        entities: vec![root, other],
        ..Default::default()
    })?;
    editor.selection.replace([ids[0]]);
    editor.begin_rotation(&mut assets)?;
    editor.preview_rotation(Vec3::X, 0.2, &mut assets)?;
    let accepted = editor.document.shared_source();
    editor.selection.replace([ids[1]]);
    assert!(editor.preview_rotation(Vec3::Y, 0.3, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    editor.cancel_transform()?;
    editor.selection.replace([ids[0]]);
    editor.begin_rotation(&mut assets)?;
    editor.preview_rotation(Vec3::X, 0.2, &mut assets)?;
    let accepted = editor.document.shared_source();
    let source = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    gltf["nodes"][1]["translation"][0] = serde_json::json!(-2.);
    std::fs::write(source, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    assert!(editor.preview_rotation(Vec3::Y, 0.3, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    editor.cancel_transform()?;
    Ok(())
}

#[test]
fn prefab_rotation_overrides_only_changed_fields_and_keeps_source_scale_inheritance() -> Result<()>
{
    let (temp, mut editor, mut assets) = fixture()?;
    let snapshot = editor.document.shared_source().expand(&mut assets)?;
    let entity = snapshot
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap();
    let id = entity.id;
    editor.selection.replace([id]);
    editor.begin_rotation(&mut assets)?;
    editor.preview_rotation(Vec3::Y, 0.3, &mut assets)?;
    let accepted = editor.document.shared_source();
    let overrides: Vec<_> = accepted
        .instances
        .iter()
        .flat_map(|instance| &instance.overrides)
        .collect();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].field, schema::ROTATION);
    let revision = editor.document.revision();
    editor.preview_rotation(Vec3::Y, 0.3, &mut assets)?;
    assert_eq!(editor.document.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(
        &accepted,
        &editor.document.shared_source()
    ));
    editor.commit()?;
    editor.drag_origin = None;
    let source = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    gltf["nodes"][1]["scale"] = serde_json::json!([2., 3., 4.]);
    std::fs::write(source, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    let expanded = editor.document.shared_source().expand(&mut assets)?;
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .transform
            .scale,
        [2., 3., 4.]
    );
    Ok(())
}

#[test]
fn legal_yaw_preserves_uniform_character_scale_and_reflection_axes() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    for scale in [10., 100., 1000.] {
        let mut character = EntityData::new("large character");
        character.transform.scale = [scale; 3];
        character.physics = Some(PhysicsBody {
            motion: BodyMotion::Kinematic,
            shape: ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.3,
            },
            ..Default::default()
        });
        character.character = Some(Character::default());
        let id = character.id;
        editor.document = Document::open(SceneSnapshot {
            entities: vec![character],
            ..Default::default()
        })?;
        editor.selection.replace([id]);
        editor.begin_rotation(&mut assets)?;
        for angle in [0.3, 0.7, 1.1, 1.9] {
            editor.preview_rotation(Vec3::Y, angle, &mut assets)?;
            assert_eq!(
                editor.document.shared_source().base.entities[0]
                    .transform
                    .scale,
                [scale; 3]
            );
        }
        editor.cancel_transform()?;
    }
    for scale in [[-2., -3., 4.], [2., -3., -4.], [-2., -3., -4.]] {
        let mut root = EntityData::new("reflected");
        root.transform.scale = scale;
        let id = root.id;
        editor.document = Document::open(SceneSnapshot {
            entities: vec![root],
            ..Default::default()
        })?;
        editor.selection.replace([id]);
        editor.begin_rotation(&mut assets)?;
        editor.preview_rotation(Vec3::Z, 0.7, &mut assets)?;
        assert_eq!(
            editor.document.shared_source().base.entities[0]
                .transform
                .scale,
            scale
        );
        editor.cancel_transform()?;
    }
    Ok(())
}

#[test]
fn cached_translation_keeps_the_accepted_revision_but_rejects_new_product_generations() -> Result<()>
{
    let (temp, mut editor, mut assets) = fixture()?;
    let root = EntityData::new("root");
    let id = root.id;
    editor.document = Document::open(SceneSnapshot {
        entities: vec![root],
        ..Default::default()
    })?;
    editor.selection.replace([id]);
    let before = editor.document.shared_source();
    editor.begin_translation(&mut assets)?;
    editor.preview_translation(Vec3::ZERO, &mut assets)?;
    assert!(std::sync::Arc::ptr_eq(
        &before,
        &editor.document.shared_source()
    ));
    editor.preview_translation(Vec3::X, &mut assets)?;
    let accepted = editor.document.shared_source();
    let revision = editor.document.revision();
    editor.preview_translation(Vec3::X, &mut assets)?;
    assert_eq!(editor.document.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(
        &accepted,
        &editor.document.shared_source()
    ));
    assert!(
        editor
            .preview_translation(Vec3::splat(f32::NAN), &mut assets)
            .is_err()
    );
    assert_eq!(
        editor.drag_origin.as_ref().unwrap().translation_preview,
        Vec3::X
    );
    let source = temp.path().join("project/assets/showcase.gltf");
    let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    gltf["nodes"][1]["translation"][0] = serde_json::json!(-2.);
    std::fs::write(source, serde_json::to_vec(&gltf)?)?;
    comet_asset_pipeline::prepare(&temp.path().join("project"), &editor.products)?;
    assets.refresh()?;
    assert!(editor.preview_translation(Vec3::X, &mut assets).is_err());
    assert_eq!(editor.document.shared_source(), accepted);
    assert_eq!(editor.document.revision(), revision);
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}
