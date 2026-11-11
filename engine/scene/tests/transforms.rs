use anyhow::Result;
use comet_core::{
    EntityId,
    math::{Mat4, Quat, Vec3},
};
use comet_scene::{
    ContentSource, EntityData, FieldUpdate, PoseUpdate, PrefabInstance, PrefabResolver, Scene,
    SceneSnapshot, Transform, checked_transform_inverse,
    schema::{self, FieldValue},
    world_transforms,
};

struct Prefab(ContentSource);
impl PrefabResolver for Prefab {
    fn prefab(&mut self, _: comet_core::AssetId) -> Result<ContentSource> {
        Ok(self.0.clone())
    }
}

#[test]
fn unordered_hierarchy_composes_rotation_reflection_scale_and_translation() -> Result<()> {
    let mut root = EntityData::new("root");
    root.transform.translation = [3., 2., -1.];
    root.transform.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
    root.transform.scale = [-2., 3., 4.];
    let mut child = EntityData::new("child");
    child.parent = Some(root.id);
    child.transform.translation = [1., 0., 0.];
    child.transform.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array();
    child.transform.scale = [1., 2., 1.];
    let mut leaf = EntityData::new("leaf");
    leaf.parent = Some(child.id);
    leaf.transform.translation = [0., 0., 1.];
    let root_id = root.id;
    let child_id = child.id;
    let leaf_id = leaf.id;
    let snapshot = SceneSnapshot {
        entities: vec![leaf, child, root],
        ..Default::default()
    };
    let matrices = world_transforms(&snapshot)?;
    assert!(
        matrices[&root_id]
            .transform_point3(Vec3::X)
            .abs_diff_eq(Vec3::new(3., 0., -1.), 1e-5)
    );
    assert!(
        matrices[&child_id]
            .transform_point3(Vec3::ZERO)
            .abs_diff_eq(Vec3::new(3., 0., -1.), 1e-5)
    );
    assert!(
        matrices[&leaf_id]
            .transform_point3(Vec3::ZERO)
            .abs_diff_eq(Vec3::new(3., -2., -1.), 1e-5)
    );
    Ok(())
}

#[test]
fn invalid_hierarchies_and_world_overflow_return_errors() {
    let mut root = EntityData::new("root");
    root.parent = Some(EntityId::new());
    let mut snapshot = SceneSnapshot {
        entities: vec![root.clone()],
        ..Default::default()
    };
    assert!(world_transforms(&snapshot).is_err());
    snapshot.entities[0].parent = Some(root.id);
    assert!(world_transforms(&snapshot).is_err());
    root.parent = None;
    snapshot.entities = vec![root.clone(), root.clone()];
    assert!(world_transforms(&snapshot).is_err());
    root.transform.scale = [f32::MAX; 3];
    let mut child = EntityData::new("overflow child");
    child.parent = Some(root.id);
    child.transform.scale = [2.; 3];
    snapshot.entities = vec![root, child];
    assert!(snapshot.validate().is_err());
    assert!(world_transforms(&snapshot).is_err());
}

#[test]
fn field_and_replace_candidates_reject_world_overflow_without_publishing() -> Result<()> {
    let mut parent = EntityData::new("large parent");
    parent.transform.scale = [1e20, 1., 1.];
    let mut child = EntityData::new("child");
    child.parent = Some(parent.id);
    let parent_id = parent.id;
    let child_id = child.id;
    let source = SceneSnapshot {
        entities: vec![parent, child],
        ..Default::default()
    };
    let mut scene = Scene::open(source.clone())?;
    let handle = scene.handle(child_id)?;
    let before = scene.shared_snapshot();
    let revision = scene.revision();
    for (field, value) in [
        (schema::TRANSLATION, FieldValue::Vec3([1e20, 0., 0.])),
        (schema::SCALE, FieldValue::Vec3([1e20, 1., 1.])),
    ] {
        let updates = [
            FieldUpdate {
                entity: parent_id,
                field: schema::TRANSLATION,
                value: FieldValue::Vec3([2., 3., 4.]),
            },
            FieldUpdate {
                entity: child_id,
                field,
                value,
            },
        ];
        let mut authored = source.clone();
        assert!(authored.set_fields(&updates).is_err());
        assert_eq!(authored, source);
        assert!(scene.set_fields(&updates).is_err());
        assert_eq!(scene.revision(), revision);
        assert!(std::sync::Arc::ptr_eq(&scene.shared_snapshot(), &before));
        assert_eq!(scene.resolve(handle)?.transform, Transform::default());
    }
    let mut invalid = source.clone();
    invalid.entities[1].transform.translation = [1e20, 0., 0.];
    assert!(Scene::open(invalid.clone()).is_err());
    assert!(scene.replace(invalid.clone()).is_err());
    assert!(SceneSnapshot::from_json(&serde_json::to_vec(&invalid)?).is_err());
    assert!(invalid.to_json().is_err());
    assert_eq!(scene.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(&scene.shared_snapshot(), &before));
    assert_eq!(scene.resolve(handle)?.transform, Transform::default());

    scene.set_fields(&[FieldUpdate {
        entity: child_id,
        field: schema::TRANSLATION,
        value: FieldValue::Vec3([1e18, 0., 0.]),
    }])?;
    let accepted = scene.shared_snapshot();
    let position = world_transforms(&accepted)?[&child_id].w_axis.x;
    assert!(position.is_finite() && position > 0.99e38 && position < 1.01e38);
    assert_eq!(scene.revision(), revision + 1);
    assert_eq!(scene.resolve(handle)?.transform.translation, [1e18, 0., 0.]);
    assert_eq!(before.entities.len(), source.entities.len());
    assert!(
        before
            .entities
            .iter()
            .all(|entity| entity.transform.translation == [0.; 3])
    );
    Ok(())
}

#[test]
fn pose_candidates_check_descendants_before_changing_revision_or_handles() -> Result<()> {
    let parent = EntityData::new("parent");
    let mut child = EntityData::new("large offset");
    child.parent = Some(parent.id);
    child.transform.translation = [2e38, 0., 0.];
    let other = EntityData::new("valid prefix");
    let parent_id = parent.id;
    let child_id = child.id;
    let other_id = other.id;
    let source = SceneSnapshot {
        entities: vec![child, other, parent],
        ..Default::default()
    };
    let mut scene = Scene::open(source.clone())?;
    let handle = scene.handle(parent_id)?;
    let before = scene.shared_snapshot();
    let revision = scene.revision();
    assert!(
        scene
            .update_poses(&[
                PoseUpdate {
                    entity: other_id,
                    translation: [2., 3., 4.],
                    rotation: [0., 0., 0., 1.],
                },
                PoseUpdate {
                    entity: parent_id,
                    translation: [2e38, 0., 0.],
                    rotation: [0., 0., 0., 1.],
                },
            ])
            .is_err()
    );
    assert_eq!(scene.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(&scene.shared_snapshot(), &before));
    assert_eq!(scene.resolve(handle)?.transform, Transform::default());

    scene.update_poses(&[PoseUpdate {
        entity: parent_id,
        translation: [1e38, 0., 0.],
        rotation: [0., 0., 0., 1.],
    }])?;
    let after = scene.shared_snapshot();
    after.validate()?;
    let position = world_transforms(&after)?[&child_id].w_axis.x;
    assert!(position.is_finite() && position > 2.99e38 && position < 3.01e38);
    assert_eq!(scene.revision(), revision + 1);
    assert_eq!(scene.resolve(handle)?.transform.translation, [1e38, 0., 0.]);
    let accepted = scene.shared_snapshot();
    scene.update_poses(&[PoseUpdate {
        entity: parent_id,
        translation: [1e38, 0., 0.],
        rotation: [0., 0., 0., 1.],
    }])?;
    assert_eq!(scene.revision(), revision + 1);
    assert!(std::sync::Arc::ptr_eq(&scene.shared_snapshot(), &accepted));
    assert!(
        before
            .entities
            .iter()
            .any(|entity| entity.id == parent_id && entity.transform == Transform::default())
    );
    assert_eq!(source.entities[0].transform.translation, [2e38, 0., 0.]);
    Ok(())
}

#[test]
fn prefab_attachment_rejects_world_overflow_without_changing_sources() -> Result<()> {
    let mut parent = EntityData::new("large parent");
    parent.transform.scale = [1e20, 1., 1.];
    let mut child = EntityData::new("prefab child");
    child.transform.translation = [1e20, 0., 0.];
    let parent_id = parent.id;
    let child_id = child.id;
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![parent],
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: comet_core::InstanceId::new(),
            prefab: comet_core::AssetId::new(),
            parent: Some(parent_id),
            overrides: vec![],
        }],
        ..Default::default()
    };
    let prefab = ContentSource {
        base: SceneSnapshot {
            entities: vec![child],
            ..Default::default()
        },
        ..Default::default()
    };
    source.validate()?;
    prefab.validate()?;
    let mut resolver = Prefab(prefab.clone());
    assert!(source.expand(&mut resolver).is_err());
    assert_eq!(resolver.0, prefab);
    let mut valid = source.clone();
    valid.instances[0]
        .overrides
        .push(comet_scene::FieldOverride {
            entity: child_id,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([1., 0., 0.]),
        });
    valid.expand(&mut resolver)?.validate()?;
    assert!(source.instances[0].overrides.is_empty());
    Ok(())
}

#[test]
fn prefab_overrides_validate_the_complete_candidate_independent_of_field_order() -> Result<()> {
    let mut parent = EntityData::new("prefab parent");
    parent.transform.scale = [1e-20, 1., 1.];
    let mut child = EntityData::new("prefab child");
    child.parent = Some(parent.id);
    child.transform.translation = [1e20, 0., 0.];
    let parent_id = parent.id;
    let child_id = child.id;
    let prefab = ContentSource {
        base: SceneSnapshot {
            entities: vec![parent, child],
            ..Default::default()
        },
        ..Default::default()
    };
    let instance = comet_core::InstanceId::new();
    let mut source = ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: comet_core::AssetId::new(),
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    let mut resolver = Prefab(prefab.clone());
    let baseline = source.expand(&mut resolver)?;
    let mut scene = Scene::open(baseline)?;
    let handle = scene.handle(child_id.in_instance(instance))?;
    let before = scene.shared_snapshot();
    let revision = scene.revision();
    source.instances[0].overrides = vec![
        comet_scene::FieldOverride {
            entity: parent_id,
            field: schema::SCALE,
            value: FieldValue::Vec3([1e20, 1., 1.]),
        },
        comet_scene::FieldOverride {
            entity: child_id,
            field: schema::TRANSLATION,
            value: FieldValue::Vec3([1e-20, 0., 0.]),
        },
    ];
    let expanded = source.expand(&mut resolver)?;
    let position = world_transforms(&expanded)?[&child_id.in_instance(instance)]
        .w_axis
        .x;
    assert!((position - 1.).abs() < 1e-5);
    source.instances[0].overrides.reverse();
    assert_eq!(source.expand(&mut resolver)?, expanded);
    assert_eq!(resolver.0, prefab);

    let mut invalid = source.clone();
    invalid.instances[0]
        .overrides
        .retain(|o| o.entity == parent_id);
    let failed_source = invalid.clone();
    assert!(
        invalid
            .expand(&mut resolver)
            .and_then(|candidate| scene.replace(candidate))
            .is_err()
    );
    assert_eq!(invalid, failed_source);
    assert_eq!(resolver.0, prefab);
    assert_eq!(scene.revision(), revision);
    assert!(std::sync::Arc::ptr_eq(&scene.shared_snapshot(), &before));
    assert_eq!(scene.resolve(handle)?.transform.translation, [1e20, 0., 0.]);
    Ok(())
}

#[test]
fn empty_content_and_degenerate_authored_scale_are_supported() -> Result<()> {
    assert!(world_transforms(&SceneSnapshot::default())?.is_empty());
    let mut root = EntityData::new("flat visual");
    root.transform.scale = [0., 1., 2.];
    let expected = root.transform.matrix();
    let id = root.id;
    let snapshot = SceneSnapshot {
        entities: vec![root],
        ..Default::default()
    };
    let matrices = world_transforms(&snapshot)?;
    assert_eq!(matrices[&id], expected);
    assert_eq!(matrices[&id].determinant(), 0.);
    Ok(())
}

#[test]
fn strict_matrix_conversion_preserves_rotated_trs_and_reflections() -> Result<()> {
    for scale in [[2., 3., 4.], [-2., 3., 4.], [2., -3., -4.]] {
        let original = Transform {
            translation: [3., -2., 1.],
            rotation: Quat::from_euler(comet_core::math::EulerRot::XYZ, 0.2, 0.7, -0.4).to_array(),
            scale,
        };
        let matrix = original.matrix();
        let restored = Transform::from_matrix(matrix)?;
        assert!(restored.matrix().abs_diff_eq(matrix, 1e-5));
        assert!((checked_transform_inverse(matrix)? * matrix).abs_diff_eq(Mat4::IDENTITY, 1e-5));
    }
    Ok(())
}

#[test]
fn strict_conversion_rejects_shear_projection_nonfinite_and_degenerate_matrices() {
    let mut shear = Mat4::IDENTITY;
    shear.y_axis.x = 0.2;
    let mut projection = Mat4::IDENTITY;
    projection.x_axis.w = 1e-6;
    let mut nonfinite = Mat4::IDENTITY;
    nonfinite.w_axis.x = f32::INFINITY;
    for matrix in [
        shear,
        projection,
        nonfinite,
        Mat4::from_scale(Vec3::new(0., 1., 1.)),
        Mat4::from_scale(Vec3::new(1e-8, 1., 1.)),
    ] {
        assert!(Transform::from_matrix(matrix).is_err(), "{matrix:?}");
    }
    assert!(checked_transform_inverse(Mat4::from_scale(Vec3::ZERO)).is_err());
}
