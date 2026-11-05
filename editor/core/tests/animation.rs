use anyhow::Result;
use comet_core::AssetId;
use comet_editor_core::Document;
use comet_scene::{
    Animator, ContentSource, EntityData, Model, PhysicsBody, PrefabResolver, SceneSnapshot,
    SkinBinding, Surface,
};
use std::{collections::BTreeMap, sync::Arc};

struct Empty;
impl PrefabResolver for Empty {
    fn prefab(&mut self, _: AssetId) -> Result<ContentSource> {
        anyhow::bail!("unexpected prefab")
    }
}

#[test]
fn component_previews_upgrade_only_the_required_protocol_and_reject_bad_animation_atomically()
-> Result<()> {
    let entity = EntityData::new("component protocols");
    let id = entity.id;
    let mut document = Document::open(SceneSnapshot {
        version: 1,
        entities: vec![entity],
    })?;
    let original = document.shared_source();
    document.begin()?;
    document.preview_physics(id, Some(PhysicsBody::default()), None, &mut Empty)?;
    assert_eq!(document.shared_source().base.version, 2);
    document.commit()?;
    document.undo()?;
    assert_eq!(document.shared_source(), original);
    document.redo()?;
    let physical = document.shared_source();
    document.begin()?;
    document.preview_physics(
        id,
        Some(PhysicsBody {
            motion: comet_scene::BodyMotion::Kinematic,
            shape: comet_scene::ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.3,
            },
            ..Default::default()
        }),
        Some(comet_scene::Character {
            crouch_speed: 1.5,
            ..Default::default()
        }),
        &mut Empty,
    )?;
    assert_eq!(document.shared_source().base.version, 3);
    document.cancel()?;
    assert!(Arc::ptr_eq(&document.shared_source(), &physical));
    let animator = Animator {
        skeleton: AssetId::new(),
        clip: None,
        bindings: BTreeMap::new(),
        looping: true,
        speed: 1.,
    };
    document.begin()?;
    document.preview_animation(id, Some(animator.clone()), None, &mut Empty)?;
    assert_eq!(document.shared_source().base.version, 9);
    let accepted = document.shared_source();
    let mut invalid = animator;
    invalid.speed = f32::NAN;
    assert!(
        document
            .preview_animation(id, Some(invalid), None, &mut Empty)
            .is_err()
    );
    assert!(Arc::ptr_eq(&document.shared_source(), &accepted));
    document.cancel()?;
    assert!(Arc::ptr_eq(&document.shared_source(), &physical));
    assert_eq!(document.history_len(), 1);
    assert_eq!(document.shared_source().base.entities[0].id, id);
    Ok(())
}

#[test]
fn removing_a_skinned_model_retires_its_skin_in_the_same_undoable_candidate() -> Result<()> {
    let mut entity = EntityData::new("unbound authored skin");
    let id = entity.id;
    entity.model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    entity.skin = Some(SkinBinding {
        skin: AssetId::new(),
        animator: None,
    });
    let mut document = Document::open(SceneSnapshot {
        version: 9,
        entities: vec![entity],
    })?;
    let original = document.shared_source();
    document.begin()?;
    document.preview_model(id, None, &mut Empty)?;
    document.commit()?;
    let removed = document.shared_source();
    assert!(removed.base.entities[0].model.is_none() && removed.base.entities[0].skin.is_none());
    document.undo()?;
    assert_eq!(document.shared_source(), original);
    assert!(!document.is_dirty());
    document.redo()?;
    assert_eq!(document.shared_source(), removed);
    Ok(())
}

#[test]
fn removing_a_controller_unbinds_its_skins_without_clearing_other_entity_links() -> Result<()> {
    let mut controller = EntityData::new("controller");
    let id = controller.id;
    controller.animator = Some(Animator {
        skeleton: AssetId::new(),
        clip: None,
        bindings: BTreeMap::new(),
        looping: true,
        speed: 1.,
    });
    let mut model = EntityData::new("skinned model");
    model.model = Some(Model {
        mesh: AssetId::new(),
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    let reference = comet_scene::EntityRef::Local { entity: id };
    model.skin = Some(SkinBinding {
        skin: AssetId::new(),
        animator: Some(reference.clone()),
    });
    model.links.target = Some(reference.clone());
    let mesh_id = model.id;
    let mut document = Document::open(SceneSnapshot {
        version: 9,
        entities: vec![controller, model],
    })?;
    let original = document.shared_source();
    document.begin()?;
    document.preview_animation(id, None, None, &mut Empty)?;
    document.commit()?;
    let current = document.shared_source();
    let mesh = current
        .base
        .entities
        .iter()
        .find(|entity| entity.id == mesh_id)
        .unwrap();
    assert!(mesh.skin.as_ref().unwrap().animator.is_none());
    assert_eq!(mesh.links.target, Some(reference));
    assert_eq!(document.history_len(), 1);
    document.undo()?;
    assert_eq!(document.shared_source(), original);
    assert!(!document.is_dirty());
    Ok(())
}
