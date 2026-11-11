use anyhow::Result;
use comet_core::{
    AssetId, EntityId, InstanceId,
    math::{Mat4, Quat, Vec3},
};
use comet_editor_core::Document;
use comet_scene::{ContentSource, EntityData, PrefabInstance, PrefabResolver, SceneSnapshot};

struct Library {
    asset: AssetId,
    source: ContentSource,
    fail: bool,
    calls: usize,
}
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.calls += 1;
        anyhow::ensure!(!self.fail && id == self.asset, "prefab unavailable");
        Ok(self.source.clone())
    }
}
fn library() -> Library {
    Library {
        asset: AssetId::new(),
        source: ContentSource {
            base: SceneSnapshot {
                entities: vec![EntityData::new("prefab root")],
                ..Default::default()
            },
            ..Default::default()
        },
        fail: false,
        calls: 0,
    }
}
fn world(snapshot: &SceneSnapshot, id: EntityId) -> Mat4 {
    let entity = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == id)
        .unwrap();
    entity
        .parent
        .map_or(Mat4::IDENTITY, |id| world(snapshot, id))
        * Mat4::from_scale_rotation_translation(
            Vec3::from_array(entity.transform.scale),
            Quat::from_array(entity.transform.rotation),
            Vec3::from_array(entity.transform.translation),
        )
}
fn assert_pose(left: Mat4, right: Mat4) {
    for (a, b) in left.to_cols_array().into_iter().zip(right.to_cols_array()) {
        assert!((a - b).abs() < 1e-4, "{left:?} differs from {right:?}");
    }
}

#[test]
fn batch_reparent_moves_top_level_roots_once_and_preserves_their_expanded_worlds() -> Result<()> {
    let mut assets = library();
    let mut old_parent = EntityData::new("old parent");
    old_parent.transform.translation = [2., -1., 3.];
    old_parent.transform.rotation = Quat::from_rotation_y(0.4).to_array();
    old_parent.transform.scale = [2.; 3];
    let mut first = EntityData::new("first root");
    first.parent = Some(old_parent.id);
    first.transform.translation = [1., 0., 0.];
    first.transform.rotation = Quat::from_rotation_z(0.2).to_array();
    first.transform.scale = [0.5, 1., 2.];
    let mut child = EntityData::new("selected descendant");
    child.parent = Some(first.id);
    child.transform.translation = [0., 1., 0.];
    let mut second = EntityData::new("second root");
    second.transform.translation = [-1., 2., 1.];
    second.transform.rotation = Quat::from_rotation_y(0.5).to_array();
    let mut target = EntityData::new("target");
    target.transform.translation = [-3., 2., -1.];
    target.transform.rotation = Quat::from_rotation_x(0.2).to_array();
    target.transform.scale = [0.5; 3];
    let (first_id, second_id, child_id, target_id) = (first.id, second.id, child.id, target.id);
    let instance = PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab: assets.asset,
        parent: Some(child.id),
        overrides: vec![],
    };
    let mut doc = Document::open_source(ContentSource {
        base: SceneSnapshot {
            entities: vec![child.clone(), target, first, second, old_parent],
            ..Default::default()
        },
        instances: vec![instance.clone()],
        ..Default::default()
    })?;
    let before = doc.shared_source();
    let expanded = before.expand(&mut assets)?;
    assets.calls = 0;
    doc.reparent_entities(
        &[child_id, first_id, second_id, first_id],
        Some(target_id),
        &mut assets,
    )?;
    assert_eq!(assets.calls, 1, "the complete group is expanded once");
    let after = doc.shared_source();
    assert_eq!(doc.history_len(), 1);
    assert_eq!(after.instances, vec![instance]);
    let find = |id| {
        after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
    };
    assert_eq!(find(first_id).parent, Some(target_id));
    assert_eq!(find(second_id).parent, Some(target_id));
    assert_eq!(
        find(child_id),
        &child,
        "a selected descendant follows its selected parent without a second local adjustment"
    );
    let updated = after.expand(&mut assets)?;
    for entity in &expanded.entities {
        assert_pose(world(&expanded, entity.id), world(&updated, entity.id));
    }
    let revision = doc.revision();
    doc.reparent_entities(
        &[first_id, child_id, first_id, second_id],
        Some(target_id),
        &mut assets,
    )?;
    doc.reparent_entities(&[], Some(EntityId::new()), &mut assets)?;
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.history_len(), 1);
    doc.undo()?;
    assert_eq!(doc.shared_source(), before);
    doc.redo()?;
    assert_eq!(doc.shared_source(), after);
    Ok(())
}

#[test]
fn invalid_batch_reparent_preserves_the_whole_group_and_redo() -> Result<()> {
    let mut assets = library();
    let first = EntityData::new("first");
    let second = EntityData::new("second");
    let mut descendant = EntityData::new("descendant");
    descendant.parent = Some(second.id);
    let target = EntityData::new("target");
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![
            first.clone(),
            second.clone(),
            descendant.clone(),
            target.clone(),
        ],
        ..Default::default()
    })?;
    doc.insert_entity(EntityData::new("redo survives"), &mut assets)?;
    doc.undo()?;
    let before = doc.shared_source();
    let revision = doc.revision();
    for (ids, parent) in [
        (vec![first.id, second.id], Some(descendant.id)),
        (vec![first.id, second.id], Some(second.id)),
        (vec![first.id, EntityId::new()], Some(target.id)),
        (vec![first.id, second.id], Some(EntityId::new())),
    ] {
        assert!(doc.reparent_entities(&ids, parent, &mut assets).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &doc.shared_source()));
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.history_len(), 0);
    }
    doc.redo()?;
    assert_eq!(doc.shared_source().base.entities.len(), 5);
    Ok(())
}

#[test]
fn one_invalid_transform_or_physics_root_rejects_an_entire_reparent_batch() -> Result<()> {
    let mut assets = library();
    for physics in [false, true] {
        let mut target = EntityData::new("target");
        target.transform.scale = if physics { [1.; 3] } else { [2., 1., 1.] };
        let valid = EntityData::new("representable root");
        let mut invalid = EntityData::new("invalid root");
        if physics {
            invalid.physics = Some(Default::default());
        } else {
            invalid.transform.rotation = Quat::from_rotation_z(0.5).to_array();
        }
        let ids = [valid.id, invalid.id];
        let target_id = target.id;
        let mut doc = Document::open(SceneSnapshot {
            entities: vec![target, valid, invalid],
            ..Default::default()
        })?;
        let before = doc.shared_source();
        let revision = doc.revision();
        assert!(
            doc.reparent_entities(&ids, Some(target_id), &mut assets)
                .is_err()
        );
        assert!(std::sync::Arc::ptr_eq(&before, &doc.shared_source()));
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.history_len(), 0);
    }
    Ok(())
}

#[test]
fn reparent_preserves_subtree_prefab_identity_world_pose_and_one_history_item() -> Result<()> {
    let mut assets = library();
    let mut parent = EntityData::new("old parent");
    parent.transform.translation = [3., -2., 4.];
    parent.transform.rotation = Quat::from_rotation_y(0.7).to_array();
    parent.transform.scale = [2.; 3];
    let mut target = EntityData::new("new parent");
    target.transform.translation = [-4., 3., 1.];
    target.transform.rotation = Quat::from_rotation_x(0.3).to_array();
    target.transform.scale = [-1.; 3];
    let mut child = EntityData::new("moving subtree");
    child.parent = Some(parent.id);
    child.transform.translation = [1., 2., 3.];
    child.transform.rotation = Quat::from_rotation_z(-0.4).to_array();
    child.transform.scale = [0.5, 1., 2.];
    let mut leaf = EntityData::new("authored leaf");
    leaf.parent = Some(child.id);
    leaf.transform.translation = [0., 2., 0.];
    let selected = child.id;
    let target_id = target.id;
    let instance = PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab: assets.asset,
        parent: Some(leaf.id),
        overrides: vec![],
    };
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![parent, target, child, leaf],
            ..Default::default()
        },
        instances: vec![instance.clone()],
        ..Default::default()
    };
    let mut doc = Document::open_source(source)?;
    let before = doc.source();
    let expanded = before.expand(&mut assets)?;
    doc.reparent_entity(selected, Some(target_id), &mut assets)?;
    assert_eq!(doc.history_len(), 1);
    assert!(doc.is_dirty());
    let after = doc.source();
    assert_eq!(after.instances, vec![instance]);
    assert_eq!(after.base.entities.len(), before.base.entities.len());
    let updated = after.expand(&mut assets)?;
    for entity in &expanded.entities {
        assert_pose(world(&expanded, entity.id), world(&updated, entity.id));
    }
    doc.undo()?;
    assert_eq!(doc.source(), before);
    assert!(!doc.is_dirty());
    doc.redo()?;
    assert_eq!(doc.source(), after);
    doc.reparent_entity(selected, None, &mut assets)?;
    assert_eq!(doc.history_len(), 2);
    assert_pose(
        world(&expanded, selected),
        world(&doc.source().base, selected),
    );
    doc.undo()?;
    assert_eq!(doc.source(), after);
    Ok(())
}

#[test]
fn invalid_reparent_keeps_source_revision_history_and_redo() -> Result<()> {
    let mut assets = library();
    let root = EntityData::new("root");
    let mut child = EntityData::new("child");
    child.parent = Some(root.id);
    let root_id = root.id;
    let child_id = child.id;
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![root, child],
        ..Default::default()
    })?;
    let addition = EntityData::new("redo target");
    doc.insert_entity(addition, &mut assets)?;
    doc.undo()?;
    let before = doc.source();
    let revision = doc.revision();
    for (id, parent) in [
        (root_id, Some(root_id)),
        (root_id, Some(child_id)),
        (root_id, Some(EntityId::new())),
        (EntityId::new(), Some(child_id)),
    ] {
        assert!(doc.reparent_entity(id, parent, &mut assets).is_err());
        assert_eq!(doc.source(), before);
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.history_len(), 0);
    }
    doc.reparent_entity(child_id, Some(root_id), &mut assets)?;
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.history_len(), 0);
    doc.redo()?;
    assert_eq!(doc.source().base.entities.len(), 3);
    Ok(())
}

#[test]
fn shear_singular_and_physics_parent_candidates_do_not_publish() -> Result<()> {
    let mut assets = library();
    for scale in [[0., 1., 1.], [1e-9, 1., 1.], [2., 1., 1.]] {
        let mut parent = EntityData::new("parent");
        parent.transform.scale = scale;
        let mut child = EntityData::new("child");
        child.transform.rotation = Quat::from_rotation_z(0.6).to_array();
        let mut doc = Document::open(SceneSnapshot {
            entities: vec![parent.clone(), child.clone()],
            ..Default::default()
        })?;
        let before = doc.source();
        let revision = doc.revision();
        assert!(
            doc.reparent_entity(child.id, Some(parent.id), &mut assets)
                .is_err()
        );
        assert_eq!(doc.source(), before);
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.history_len(), 0);
    }
    let parent = EntityData::new("parent");
    let mut physical = EntityData::new("physical root");
    physical.physics = Some(Default::default());
    let mut doc = Document::open(SceneSnapshot {
        entities: vec![parent.clone(), physical.clone()],
        ..Default::default()
    })?;
    let before = doc.source();
    assert!(
        doc.reparent_entity(physical.id, Some(parent.id), &mut assets)
            .is_err()
    );
    assert_eq!(doc.source(), before);
    assert_eq!(doc.history_len(), 0);
    Ok(())
}

#[test]
fn prefab_interiors_and_failed_expansion_cannot_change_authored_structure() -> Result<()> {
    let mut assets = library();
    let container = EntityData::new("container");
    let parent = EntityData::new("parent");
    let instance = PrefabInstance {
        translation: [0.; 3],
        id: InstanceId::new(),
        prefab: assets.asset,
        parent: Some(container.id),
        overrides: vec![],
    };
    let interior = assets.source.base.entities[0].id.in_instance(instance.id);
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![container.clone(), parent.clone()],
            ..Default::default()
        },
        instances: vec![instance],
        ..Default::default()
    };
    let mut doc = Document::open_source(source)?;
    let before = doc.source();
    for (id, target) in [(interior, Some(parent.id)), (parent.id, Some(interior))] {
        assert!(doc.reparent_entity(id, target, &mut assets).is_err());
        assert_eq!(doc.source(), before);
    }
    assets.fail = true;
    assert!(
        doc.reparent_entity(container.id, Some(parent.id), &mut assets)
            .is_err()
    );
    assert_eq!(doc.source(), before);
    assert_eq!(doc.history_len(), 0);
    assets.fail = false;
    doc.begin()?;
    assert!(
        doc.reparent_entity(container.id, Some(parent.id), &mut assets)
            .is_err()
    );
    assert!(doc.is_editing());
    doc.cancel()?;
    assert_eq!(doc.source(), before);
    Ok(())
}
