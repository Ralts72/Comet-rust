//! Character feedback drives controller parameters; the animation sampler never moves physics bodies.
use super::{AnimationCommand, TransformClaims, playback::Playback};
use anyhow::{Result, ensure};
use comet_asset_data::AnimationParameter as Parameter;
use comet_core::{EntityId, math::Quat};
use comet_scene::{
    ColliderShape, EntityRef, FieldUpdate, SceneSnapshot,
    schema::{self, FieldValue},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Driver {
    pub owner: EntityId,
    character: EntityId,
    animator: EntityId,
    half_height: f32,
    crouch_ratio: f32,
}
pub(super) fn prepare(
    snapshot: &SceneSnapshot,
    playbacks: &BTreeMap<EntityId, Playback>,
    claims: &TransformClaims,
    unbound: &mut BTreeSet<EntityId>,
) -> Result<Vec<Driver>> {
    let entities: BTreeMap<_, _> = snapshot.entities.iter().map(|e| (e.id, e)).collect();
    let mut driven = BTreeSet::new();
    let mut drivers = Vec::new();
    for owner in &snapshot.entities {
        let Some(driver) = &owner.character_animation else {
            continue;
        };
        let (Some(EntityRef::Local { entity: actor }), Some(EntityRef::Local { entity: animator })) =
            (&driver.character, &driver.animator)
        else {
            unbound.insert(owner.id);
            continue;
        };
        let actor = entities[actor];
        let Some(character) = &actor.character else {
            unbound.insert(owner.id);
            continue;
        };
        if owner.parent != Some(actor.id) {
            unbound.insert(owner.id);
            continue;
        }
        let Some(playback) = playbacks.get(animator) else {
            unbound.insert(owner.id);
            continue;
        };
        let mut ancestor = Some(*animator);
        while let Some(id) = ancestor {
            if id == owner.id {
                break;
            }
            ancestor = entities[&id].parent;
        }
        ensure!(
            ancestor == Some(owner.id),
            "character animator must descend from its visual owner"
        );
        for (key, value) in [
            ("speed", Parameter::Number(0.)),
            ("grounded", Parameter::Bool(false)),
            ("rising", Parameter::Bool(false)),
            ("crouched", Parameter::Bool(false)),
        ] {
            ensure!(
                playback
                    .control
                    .parameters
                    .get(key)
                    .is_some_and(|v| v.same_type(value)),
                "character animation controller needs correctly typed speed/grounded/rising/crouched parameters"
            );
        }
        ensure!(
            driven.insert(*animator),
            "multiple character drivers target the same animator"
        );
        ensure!(
            owner.physics.is_none()
                && owner.character.is_none()
                && owner.navigation_region.is_none(),
            "character animation visual cannot own physics or navigation"
        );
        ensure!(
            !claims.contains(&(owner.id, 1)) && !claims.contains(&(owner.id, 2)),
            "character animation visual is already driven by a clip"
        );
        let ColliderShape::Capsule {
            half_height,
            radius,
        } = actor
            .physics
            .as_ref()
            .expect("validated character body")
            .shape
        else {
            unreachable!("validated character capsule")
        };
        drivers.push(Driver {
            owner: owner.id,
            character: actor.id,
            animator: *animator,
            half_height: half_height + radius,
            crouch_ratio: character
                .crouch_height_ratio
                .max(radius / (half_height + radius)),
        });
    }
    Ok(drivers)
}
pub(super) fn drive(
    drivers: &[Driver],
    snapshot: &SceneSnapshot,
    physics: &crate::physics::Physics,
) -> Result<(Vec<AnimationCommand>, Vec<FieldUpdate>)> {
    let mut commands = Vec::new();
    let mut updates = Vec::new();
    for driver in drivers {
        let state = physics
            .character(driver.character)
            .expect("validated driver character");
        let velocity = state.horizontal_velocity;
        let speed = velocity[0].hypot(velocity[1]);
        for (name, value) in [
            ("speed", Parameter::Number(speed)),
            ("grounded", Parameter::Bool(state.grounded)),
            ("rising", Parameter::Bool(state.vertical_velocity > 0.)),
            ("crouched", Parameter::Bool(state.crouched)),
        ] {
            commands.push(AnimationCommand::Parameter(
                driver.animator,
                name.into(),
                value,
            ));
        }
        let owner = snapshot
            .entities
            .iter()
            .find(|e| e.id == driver.owner)
            .expect("validated driver visual");
        let mut translation = owner.transform.translation;
        translation[1] = -driver.half_height
            * if state.crouched {
                driver.crouch_ratio
            } else {
                1.
            };
        if translation != owner.transform.translation {
            updates.push(FieldUpdate {
                entity: driver.owner,
                field: schema::TRANSLATION,
                value: FieldValue::Vec3(translation),
            });
        }
        if speed > 0.01 {
            let rotation = Quat::from_rotation_y((-velocity[0]).atan2(-velocity[1])).to_array();
            if rotation != owner.transform.rotation {
                updates.push(FieldUpdate {
                    entity: driver.owner,
                    field: schema::ROTATION,
                    value: FieldValue::Quat(rotation),
                });
            }
        }
    }
    Ok((commands, updates))
}
