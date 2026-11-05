//! Author settings become private Rapier joints, keyed by owner and component kind.
use super::Body;
use anyhow::{Context, Result, bail, ensure};
use comet_core::EntityId;
use comet_scene::{
    EntityData, EntityRef, JointConnection, JointFrame, JointLimits, JointMotor, JointSettings,
    SceneSnapshot,
};
use rapier3d::prelude::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JointKind {
    Point,
    Hinge,
    Slider,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointStatus {
    Active,
    Disabled,
    Unbound,
}

#[derive(Default)]
pub(super) struct Joints {
    entries: BTreeMap<(EntityId, JointKind), JointStatus>,
    world_anchor: Option<RigidBodyHandle>,
}

struct Prepared<'a> {
    kind: JointKind,
    settings: &'a JointSettings,
    data: GenericJoint,
}

fn frame(frame: &JointFrame) -> Pose {
    super::pose(frame.anchor, frame.rotation)
}

fn wrapped_angle(radians: f64) -> f32 {
    ((radians + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI)
        as f32
}

fn axis_joint(
    kind: JointKind,
    owner: &JointFrame,
    connected: &JointFrame,
    limits: Option<&JointLimits>,
    motor: Option<&JointMotor>,
) -> GenericJoint {
    let (locked, axis) = match kind {
        JointKind::Hinge => (JointAxesMask::LOCKED_REVOLUTE_AXES, JointAxis::AngX),
        JointKind::Slider => (JointAxesMask::LOCKED_PRISMATIC_AXES, JointAxis::LinX),
        JointKind::Point => unreachable!("point joints have no driven axis"),
    };
    let mut data = GenericJoint::new(locked);
    // Body 1 is connected, body 2 is the owner: positive motion is owner-relative.
    data.local_frame1 = frame(connected);
    data.local_frame2 = frame(owner);
    if let Some(limits) = limits {
        let range = if kind == JointKind::Hinge {
            // Rapier centres angular intervals in f32. Keep equivalent author ranges
            // near zero so a large offset cannot overflow that centre calculation.
            let half = (f64::from(limits.max) - f64::from(limits.min)) * 0.5;
            let centre = f64::from(wrapped_angle(f64::from(limits.min) + half));
            [(centre - half) as f32, (centre + half) as f32]
        } else {
            [limits.min, limits.max]
        };
        data.set_limits(axis, range);
    }
    if let Some(motor) = motor {
        let effort = match *motor {
            JointMotor::Velocity { speed, max_effort } => {
                // One microstep time constant; four unconstrained microsteps remove 15/16
                // of a velocity error. max_effort still caps the physical force/torque.
                let response = super::PHYSICS_SUBSTEPS as f32 / crate::FIXED_DELTA as f32;
                data.set_motor_velocity(axis, speed, response);
                max_effort
            }
            JointMotor::Position {
                target,
                frequency_hz,
                damping_ratio,
                max_effort,
            } => {
                let omega = std::f64::consts::TAU * f64::from(frequency_hz);
                let target = limits.map_or(target, |limits| target.clamp(limits.min, limits.max));
                let target = if kind == JointKind::Hinge {
                    wrapped_angle(f64::from(target))
                } else {
                    target
                };
                data.set_motor_position(
                    axis,
                    target,
                    (omega * omega) as f32,
                    (2. * f64::from(damping_ratio) * omega) as f32,
                );
                max_effort
            }
        };
        data.set_motor_model(axis, MotorModel::AccelerationBased);
        data.set_motor_max_force(axis, effort);
    }
    data
}

impl Joints {
    pub(super) fn new(
        world: &mut PhysicsWorld,
        bodies: &BTreeMap<EntityId, Body>,
        snapshot: &SceneSnapshot,
    ) -> Result<Self> {
        if snapshot.entities.iter().all(|entity| {
            entity.point_joint.is_none()
                && entity.hinge_joint.is_none()
                && entity.slider_joint.is_none()
        }) {
            return Ok(Self::default());
        }
        let entities: BTreeMap<_, _> = snapshot.entities.iter().map(|e| (e.id, e)).collect();
        let mut joints = Self::default();
        for &entity in entities.values() {
            if let Some(point) = &entity.point_joint {
                let mut data = GenericJoint::new(JointAxesMask::LOCKED_SPHERICAL_AXES);
                data.local_frame1.translation = Vector::from_array(point.connected_anchor);
                data.local_frame2.translation = Vector::from_array(point.anchor);
                joints.insert(
                    world,
                    bodies,
                    &entities,
                    entity,
                    Prepared {
                        kind: JointKind::Point,
                        settings: &point.settings,
                        data,
                    },
                )?;
            }
            if let Some(hinge) = &entity.hinge_joint {
                joints.insert(
                    world,
                    bodies,
                    &entities,
                    entity,
                    Prepared {
                        kind: JointKind::Hinge,
                        settings: &hinge.settings,
                        data: axis_joint(
                            JointKind::Hinge,
                            &hinge.frame,
                            &hinge.connected_frame,
                            hinge.limits.as_ref(),
                            hinge.motor.as_ref(),
                        ),
                    },
                )?;
            }
            if let Some(slider) = &entity.slider_joint {
                joints.insert(
                    world,
                    bodies,
                    &entities,
                    entity,
                    Prepared {
                        kind: JointKind::Slider,
                        settings: &slider.settings,
                        data: axis_joint(
                            JointKind::Slider,
                            &slider.frame,
                            &slider.connected_frame,
                            slider.limits.as_ref(),
                            slider.motor.as_ref(),
                        ),
                    },
                )?;
            }
        }
        Ok(joints)
    }

    fn insert(
        &mut self,
        world: &mut PhysicsWorld,
        bodies: &BTreeMap<EntityId, Body>,
        entities: &BTreeMap<EntityId, &EntityData>,
        owner: &EntityData,
        prepared: Prepared<'_>,
    ) -> Result<()> {
        let Prepared {
            kind,
            settings,
            mut data,
        } = prepared;
        let mut status = if settings.enabled {
            JointStatus::Unbound
        } else {
            JointStatus::Disabled
        };
        if settings.enabled && settings.connection != JointConnection::Unbound {
            let owner_body = bodies.get(&owner.id).context("joint owner has no body")?;
            let (connected, scale) = match &settings.connection {
                JointConnection::World => (
                    *self
                        .world_anchor
                        .get_or_insert_with(|| world.bodies.insert(RigidBodyBuilder::fixed())),
                    [1.; 3],
                ),
                JointConnection::Body(EntityRef::Local { entity }) => (
                    bodies
                        .get(entity)
                        .context("connected entity has no body")?
                        .handle,
                    entities[entity].transform.scale,
                ),
                JointConnection::Body(
                    EntityRef::External { .. } | EntityRef::InInstance { .. },
                ) => {
                    bail!(
                        "joint {} cannot connect bodies in different runtime worlds",
                        owner.id
                    )
                }
                JointConnection::Unbound => unreachable!(),
            };
            ensure!(
                world.bodies[owner_body.handle].is_dynamic()
                    || world.bodies[connected].is_dynamic(),
                "joint {} requires at least one dynamic body",
                owner.id
            );
            data.local_frame1.translation *= Vector::from_array(scale);
            data.local_frame2.translation *= Vector::from_array(owner.transform.scale);
            for (handle, local) in [
                (connected, data.local_frame1),
                (owner_body.handle, data.local_frame2),
            ] {
                let anchor = world.bodies[handle].position() * local;
                ensure!(
                    anchor.translation.is_finite() && anchor.rotation.is_finite(),
                    "joint {} frame exceeds finite physics coordinates",
                    owner.id
                );
            }
            data.contacts_enabled = settings.contacts_enabled;
            world
                .impulse_joints
                .insert(connected, owner_body.handle, data, true);
            status = JointStatus::Active;
        }
        self.entries.insert((owner.id, kind), status);
        Ok(())
    }

    pub(super) fn status(&self, entity: EntityId, kind: JointKind) -> Option<JointStatus> {
        self.entries.get(&(entity, kind)).copied()
    }
}
