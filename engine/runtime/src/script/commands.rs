use crate::{
    animation::{Animation, AnimationCommand},
    audio::{Audio, AudioCommand},
    hud::{Hud, HudCommand},
    navigation::{Navigation, NavigationCommand},
    physics::{Physics, PhysicsCommand},
    progress::{Progress, ProgressCommand},
};
use anyhow::{Context, Result, ensure};
use comet_core::{
    EntityId, ProgressValue,
    math::{Quat, Vec3},
};
use comet_scene::{
    FieldUpdate, PoseUpdate, Scene,
    schema::{self, FieldValue},
};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub(super) enum Command {
    Position(EntityId, [f32; 3]),
    Translate(EntityId, [f32; 3]),
    Rotation(EntityId, [f32; 4]),
    RotateLocal(EntityId, [f32; 3], f32),
    Scale(EntityId, [f32; 3]),
    Roughness(EntityId, f32),
    Teleport(EntityId, [f32; 3]),
    Velocity(EntityId, [f32; 3]),
    Impulse(EntityId, [f32; 3]),
    NavigateTo(EntityId, [f32; 3]),
    FollowNavigation(EntityId, EntityId),
    StopNavigation(EntityId),
    PlaySound(EntityId),
    StopSound(EntityId),
    SoundVolume(EntityId, f32),
    SetHud(EntityId, Arc<str>),
    ClearHud(EntityId),
    PlayAnimation(EntityId),
    PauseAnimation(EntityId),
    ResumeAnimation(EntityId),
    AnimationSpeed(EntityId, f32),
    AnimationParameter(EntityId, String, comet_asset_data::AnimationParameter),
    SetProgress(EntityId, String, ProgressValue),
    RemoveProgress(EntityId, String),
}

/// A callback publishes only after every world and presentation command is valid.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply(
    commands: &[Command],
    scene: &mut Scene,
    physics: &mut Physics,
    mut navigation: Option<&mut Navigation>,
    audio: &mut Audio,
    hud: &mut Hud,
    animation: &mut Animation,
    progress: &mut Progress,
) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }
    let snapshot = scene.shared_snapshot();
    let mut candidates = BTreeMap::new();
    let mut fields = BTreeMap::new();
    let mut physical = Vec::new();
    let mut navigational = Vec::new();
    let mut sounds = Vec::new();
    let mut labels = Vec::new();
    let mut animations = Vec::new();
    let mut values = Vec::new();
    for command in commands {
        let id = match *command {
            Command::SetProgress(id, ref key, ref value) => {
                values.push(ProgressCommand::Set(id, key.clone(), value.clone()));
                continue;
            }
            Command::RemoveProgress(id, ref key) => {
                values.push(ProgressCommand::Remove(id, key.clone()));
                continue;
            }
            Command::Position(id, _)
            | Command::Translate(id, _)
            | Command::Rotation(id, _)
            | Command::RotateLocal(id, _, _)
            | Command::Scale(id, _)
            | Command::Roughness(id, _)
            | Command::Teleport(id, _) => id,
            Command::Velocity(id, value) => {
                physical.push(PhysicsCommand::Velocity(id, value));
                continue;
            }
            Command::Impulse(id, value) => {
                physical.push(PhysicsCommand::Impulse(id, value));
                continue;
            }
            Command::NavigateTo(id, point) => {
                navigational.push(NavigationCommand::NavigateTo(id, point));
                continue;
            }
            Command::FollowNavigation(id, target) => {
                navigational.push(NavigationCommand::Follow(id, target));
                continue;
            }
            Command::StopNavigation(id) => {
                navigational.push(NavigationCommand::Stop(id));
                continue;
            }
            Command::PlaySound(id) => {
                sounds.push(AudioCommand::Play(id));
                continue;
            }
            Command::StopSound(id) => {
                sounds.push(AudioCommand::Stop(id));
                continue;
            }
            Command::SoundVolume(id, gain) => {
                sounds.push(AudioCommand::Gain(id, gain));
                continue;
            }
            Command::SetHud(id, ref text) => {
                labels.push(HudCommand::Set(id, text.clone()));
                continue;
            }
            Command::ClearHud(id) => {
                labels.push(HudCommand::Clear(id));
                continue;
            }
            Command::PlayAnimation(id) => {
                animations.push(AnimationCommand::Play(id));
                continue;
            }
            Command::PauseAnimation(id) => {
                animations.push(AnimationCommand::Pause(id));
                continue;
            }
            Command::ResumeAnimation(id) => {
                animations.push(AnimationCommand::Resume(id));
                continue;
            }
            Command::AnimationSpeed(id, speed) => {
                animations.push(AnimationCommand::Speed(id, speed));
                continue;
            }
            Command::AnimationParameter(id, ref name, value) => {
                animations.push(AnimationCommand::Parameter(id, name.clone(), value));
                continue;
            }
        };
        if matches!(
            command,
            Command::Position(..)
                | Command::Translate(..)
                | Command::Rotation(..)
                | Command::RotateLocal(..)
                | Command::Scale(..)
        ) {
            let field = match command {
                Command::Position(..) | Command::Translate(..) => schema::TRANSLATION,
                Command::Rotation(..) | Command::RotateLocal(..) => schema::ROTATION,
                Command::Scale(..) => schema::SCALE,
                _ => unreachable!(),
            };
            ensure!(
                !animation.is_controlled(id, field),
                "animation owns this transform field"
            );
            ensure!(
                !physics.contains(id),
                "physics owns this transform; use a physics command"
            );
            ensure!(
                snapshot
                    .entities
                    .iter()
                    .find(|e| e.id == id)
                    .is_none_or(|e| e.navigation_region.is_none()),
                "navigation region transforms are immutable during Play"
            );
        }
        // Keep only touched entities, and validate each queued write before the final publication.
        let entity = match candidates.entry(id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let index = snapshot
                    .entities
                    .binary_search_by_key(&id, |e| e.id)
                    .map_err(|_| anyhow::anyhow!("stale script entity"))?;
                entry.insert(snapshot.entities[index].clone())
            }
        };
        let (field, value) = match *command {
            Command::Position(_, value) | Command::Translate(_, value) => {
                let position = if matches!(command, Command::Translate(..)) {
                    (Vec3::from_array(entity.transform.translation) + Vec3::from_array(value))
                        .to_array()
                } else {
                    value
                };
                (schema::TRANSLATION, FieldValue::Vec3(position))
            }
            Command::Rotation(_, value) => (schema::ROTATION, FieldValue::Quat(value)),
            Command::RotateLocal(_, axis, radians) => {
                let value = if radians == 0. {
                    entity.transform.rotation
                } else {
                    (Quat::from_array(entity.transform.rotation)
                        * Quat::from_axis_angle(Vec3::from_array(axis), radians))
                    .normalize()
                    .to_array()
                };
                (schema::ROTATION, FieldValue::Quat(value))
            }
            Command::Scale(_, value) => (schema::SCALE, FieldValue::Vec3(value)),
            Command::Roughness(_, value) => (schema::ROUGHNESS, FieldValue::Scalar(value)),
            Command::Teleport(_, translation) => {
                physical.push(PhysicsCommand::Teleport(PoseUpdate {
                    entity: id,
                    translation,
                    rotation: entity.transform.rotation,
                }));
                (schema::TRANSLATION, FieldValue::Vec3(translation))
            }
            Command::Velocity(..)
            | Command::Impulse(..)
            | Command::NavigateTo(..)
            | Command::FollowNavigation(..)
            | Command::StopNavigation(..) => unreachable!(),
            Command::PlaySound(..) | Command::StopSound(..) | Command::SoundVolume(..) => {
                unreachable!()
            }
            Command::SetHud(..)
            | Command::ClearHud(..)
            | Command::SetProgress(..)
            | Command::RemoveProgress(..) => unreachable!(),
            Command::PlayAnimation(..)
            | Command::PauseAnimation(..)
            | Command::ResumeAnimation(..)
            | Command::AnimationSpeed(..)
            | Command::AnimationParameter(..) => unreachable!(),
        };
        schema::descriptor(field)?.write(entity, value.clone())?;
        fields.insert(
            (id, field.component, field.field),
            FieldUpdate {
                entity: id,
                field,
                value,
            },
        );
    }
    physics.validate_commands(&physical)?;
    audio.validate_commands(&sounds)?;
    let labels = hud.prepare(&labels)?;
    let animations = animation.prepare_commands(&animations)?;
    let values = progress.prepare(&values)?;
    if !navigational.is_empty() {
        navigation
            .as_ref()
            .context("navigation is not available in this runtime world")?
            .validate_commands(&snapshot, &navigational)?;
    }
    scene.set_fields(&fields.into_values().collect::<Vec<_>>())?;
    // No target or component can change between validation and publication on this owner thread.
    for command in physical {
        match command {
            PhysicsCommand::Teleport(update) => physics.teleport(&update)?,
            PhysicsCommand::Velocity(id, value) => physics.set_velocity(id, value)?,
            PhysicsCommand::Impulse(id, value) => physics.apply_impulse(id, value)?,
        }
    }
    if let Some(navigation) = navigation.as_mut() {
        navigation.apply_commands(&navigational);
    }
    audio.apply_commands(&sounds);
    hud.publish(labels);
    animation.publish_commands(animations);
    progress.publish(values);
    Ok(())
}
