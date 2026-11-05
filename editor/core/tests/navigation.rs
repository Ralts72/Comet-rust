use anyhow::{Context, Result};
use comet_core::{AssetId, InstanceId};
use comet_editor_core::{Document, FieldEdit, FieldTarget};
use comet_scene::{
    BodyMotion, Character, ColliderShape, ContentSource, EntityData, EntityRef, NavigationAgent,
    NavigationRegion, PhysicsBody, PrefabInstance, PrefabResolver, SceneSnapshot,
    schema::{self, FieldValue},
};
use std::{collections::BTreeMap, sync::Arc};
struct Library(BTreeMap<AssetId, ContentSource>);
impl PrefabResolver for Library {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        self.0.get(&id).cloned().context("missing prefab")
    }
}
#[test]
fn navigation_component_candidates_upgrade_as_one_history_item_and_fail_without_losing_preview()
-> Result<()> {
    let region = EntityData::new("region");
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
    let (region_id, npc_id) = (region.id, npc.id);
    let mut doc = Document::open(SceneSnapshot {
        version: 2,
        entities: vec![region, npc],
    })?;
    let before = doc.shared_source();
    let mut library = Library(BTreeMap::new());
    doc.begin()?;
    doc.preview_navigation(
        region_id,
        Some(NavigationRegion::default()),
        None,
        &mut library,
    )?;
    doc.preview_navigation(
        npc_id,
        None,
        Some(NavigationAgent {
            target: Some(EntityRef::local(region_id)),
            ..Default::default()
        }),
        &mut library,
    )?;
    let accepted = doc.shared_source();
    let revision = doc.revision();
    assert!(
        doc.preview_navigation(
            npc_id,
            Some(NavigationRegion::default()),
            None,
            &mut library
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    assert_eq!(doc.revision(), revision);
    assert!(
        doc.preview_physics(npc_id, None, None, &mut library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    doc.commit()?;
    assert_eq!(doc.source().base.version, 6);
    assert_eq!(doc.history_len(), 1);
    doc.undo()?;
    assert!(Arc::ptr_eq(&before, &doc.shared_source()));
    doc.redo()?;
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    Ok(())
}

#[test]
fn prefab_navigation_fields_keep_optional_asset_overrides_and_noop_round_trips() -> Result<()> {
    let prefab = AssetId::new();
    let region = EntityData {
        navigation_region: Some(NavigationRegion::default()),
        ..EntityData::new("region")
    };
    let id = region.id;
    let mut library = Library(BTreeMap::from([(
        prefab,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![region],
                ..Default::default()
            },
            ..Default::default()
        },
    )]));
    let instance = InstanceId::new();
    let mut doc = Document::open_source(ContentSource {
        base: SceneSnapshot {
            version: 5,
            entities: vec![],
        },
        instances: vec![PrefabInstance {
            id: instance,
            prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    let before = doc.shared_source();
    let asset = AssetId::new();
    doc.begin()?;
    for value in [Some(asset), None] {
        doc.preview_field_values(
            &[FieldEdit {
                target: FieldTarget::Prefab {
                    instance,
                    entity: id,
                },
                field: schema::NAVIGATION_DATA,
                value: FieldValue::OptionalAsset(value),
            }],
            &mut library,
        )?;
    }
    doc.commit()?;
    assert!(Arc::ptr_eq(&before, &doc.shared_source()));
    assert_eq!(doc.history_len(), 0);
    doc.begin()?;
    doc.preview_fields(
        &[FieldEdit {
            target: FieldTarget::Prefab {
                instance,
                entity: id,
            },
            field: schema::NAVIGATION_DATA,
            value: FieldValue::OptionalAsset(None),
        }],
        &mut library,
    )?;
    doc.commit()?;
    assert_eq!(doc.source().base.version, 6);
    assert_eq!(doc.source().instances[0].overrides.len(), 1);
    let accepted = doc.shared_source();
    doc.begin()?;
    assert!(
        doc.preview_navigation(id.in_instance(instance), None, None, &mut library)
            .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &doc.shared_source()));
    doc.cancel()?;
    doc.expand(&mut library)?;
    Ok(())
}
