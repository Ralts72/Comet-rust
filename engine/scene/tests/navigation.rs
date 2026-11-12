use anyhow::{Context, Result};
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{
    BodyMotion, Character, ColliderShape, ContentSource, EntityData, EntityRef, FieldOverride,
    NavigationAgent, NavigationRegion, PhysicsBody, PrefabInstance, PrefabResolver, Scene,
    SceneSnapshot,
    schema::{self, FieldValue},
};
use std::collections::BTreeMap;

struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.0.get(&id).cloned().context("missing prefab")
    }
}
fn fixture() -> SceneSnapshot {
    let mut region = EntityData::new("region");
    region.navigation_region = Some(NavigationRegion::default());
    let target = EntityData::new("target");
    let mut npc = EntityData::new("npc");
    npc.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    npc.character = Some(Character {
        player_controlled: false,
        ..Default::default()
    });
    npc.navigation_agent = Some(NavigationAgent {
        target: Some(EntityRef::local(target.id)),
        ..Default::default()
    });
    SceneSnapshot {
        entities: vec![region, npc, target],
        ..Default::default()
    }
}

#[test]
fn navigation_ecs_and_invalid_fields_preserve_atomic_publication() -> Result<()> {
    let snapshot = fixture();
    snapshot.validate()?;
    let mut scene = Scene::open(snapshot.clone())?;
    assert_eq!(scene.shared_snapshot().to_json()?, snapshot.to_json()?);
    let mut bad = snapshot.clone();
    let region = bad.entities[0].id;
    let npc = bad.entities[1].id;
    let before = bad.clone();
    assert!(
        bad.set_field(
            &[region],
            schema::NAVIGATION_HALF_EXTENTS,
            FieldValue::Vec3([1., -1., 1.])
        )
        .is_err()
    );
    assert_eq!(bad, before);
    assert!(
        bad.set_field(
            &[npc],
            schema::NAVIGATION_ARRIVAL_DISTANCE,
            FieldValue::Scalar(f32::NAN)
        )
        .is_err()
    );
    assert_eq!(bad, before);
    bad.entities[0].transform.rotation = [0., 0.5, 0., 0.8660254];
    assert!(bad.validate().is_err());
    bad = before.clone();
    bad.entities[0].parent = Some(npc);
    assert!(bad.validate().is_err());
    bad = before.clone();
    bad.entities[1]
        .character
        .as_mut()
        .unwrap()
        .player_controlled = true;
    assert!(bad.validate().is_err());
    bad = before.clone();
    bad.entities.push({
        let mut duplicate = bad.entities[0].clone();
        duplicate.id = EntityId::new();
        duplicate
    });
    assert!(bad.validate().is_err());
    Ok(())
}

#[test]
fn navigation_targets_remap_on_copy_and_clear_on_delete_while_asset_identity_stays_global()
-> Result<()> {
    let mut snapshot = fixture();
    let region = snapshot.entities[0].id;
    let npc = snapshot.entities[1].id;
    let target = snapshot.entities[2].id;
    let data = AssetId::new();
    snapshot.set_field(
        &[region],
        schema::NAVIGATION_DATA,
        FieldValue::OptionalAsset(Some(data)),
    )?;
    assert!(snapshot.asset_dependencies().contains(&data));
    let mapping = snapshot.clone_subtrees(&[npc, target])?;
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|e| e.id == mapping[&npc])
            .unwrap()
            .navigation_agent
            .as_ref()
            .unwrap()
            .target,
        Some(EntityRef::local(mapping[&target]))
    );
    snapshot.remove_subtrees(&[target])?;
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|e| e.id == npc)
            .unwrap()
            .navigation_agent
            .as_ref()
            .unwrap()
            .target,
        None
    );
    assert!(snapshot.asset_dependencies().contains(&data));
    snapshot.set_field(
        &[region],
        schema::NAVIGATION_DATA,
        FieldValue::OptionalAsset(None),
    )?;
    assert!(!snapshot.asset_dependencies().contains(&data));
    Ok(())
}

#[test]
fn prefab_navigation_overrides_keep_source_references_and_root_constraints() -> Result<()> {
    let snapshot = fixture();
    let region = snapshot.entities[0].id;
    let npc = snapshot.entities[1].id;
    let target = snapshot.entities[2].id;
    let prefab = AssetId::new();
    let data = AssetId::new();
    let mut library = Library(BTreeMap::from([(
        prefab,
        ContentSource {
            base: snapshot,
            ..Default::default()
        },
    )]));
    let roots = library.prefab(prefab)?.prefab_roots(&mut library)?;
    assert_eq!(roots.iter().filter(|r| r.requires_scene_root()).count(), 2);
    let id = InstanceId::new();
    let mut source = ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id,
            prefab,
            parent: None,
            overrides: vec![
                FieldOverride {
                    entity: region,
                    field: schema::NAVIGATION_DATA,
                    value: FieldValue::OptionalAsset(Some(data)),
                },
                FieldOverride {
                    entity: npc,
                    field: schema::NAVIGATION_TARGET,
                    value: FieldValue::Entity(Some(EntityRef::local(target))),
                },
            ],
        }],
        ..Default::default()
    };
    assert!(source.dependencies().contains(&data));
    let expanded = source.expand(&mut library)?;
    assert_eq!(
        expanded
            .entities
            .iter()
            .find(|e| e.id == npc.in_instance(id))
            .unwrap()
            .navigation_agent
            .as_ref()
            .unwrap()
            .target,
        Some(EntityRef::local(target.in_instance(id)))
    );
    assert!(roots.iter().all(|r| {
        expanded
            .entities
            .iter()
            .any(|e| e.id == r.in_instance(id) && e.parent.is_none())
    }));
    let mount = EntityData::new("invalid container");
    source.instances[0].parent = Some(mount.id);
    source.base.entities.push(mount);
    assert!(source.expand(&mut library).is_err());
    Ok(())
}
