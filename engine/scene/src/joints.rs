//! Persistent joint configuration. Angles use radians; runtime owns solver state.
use crate::{BodyMotion, EntityData, EntityRef, SceneSnapshot};
use anyhow::{Context, Result, ensure};
use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum JointConnection {
    #[default]
    World,
    Body(EntityRef),
    /// A deliberately disconnected or deleted body never becomes a world attachment.
    Unbound,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointSettings {
    pub enabled: bool,
    pub connection: JointConnection,
    pub contacts_enabled: bool,
}
impl Default for JointSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            connection: JointConnection::World,
            contacts_enabled: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointFrame {
    /// Body-local anchor before entity scale, or a world point for a world connection.
    pub anchor: [f32; 3],
    /// xyzw orientation; X is the principal axis and Y defines the zero-angle reference.
    /// Directions rotate with the body but do not inherit its scale.
    pub rotation: [f32; 4],
}
impl Default for JointFrame {
    fn default() -> Self {
        Self {
            anchor: [0.; 3],
            rotation: [0., 0., 0., 1.],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointLimits {
    /// Radians for a hinge, metres for a slider. Equal endpoints lock the free axis.
    /// A hinge range may sit anywhere on the circle, with a span of at most one turn.
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum JointMotor {
    Velocity {
        /// Radians/s for a hinge, metres/s for a slider.
        speed: f32,
        /// N·m for a hinge, N for a slider; zero applies no effort.
        max_effort: f32,
    },
    Position {
        /// Radians for a hinge, metres for a slider; runtime clamps to active hard limits.
        /// Hinge positions wrap around the circle and do not count revolutions.
        target: f32,
        frequency_hz: f32,
        damping_ratio: f32,
        max_effort: f32,
    },
}

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointJoint {
    pub settings: JointSettings,
    pub anchor: [f32; 3],
    pub connected_anchor: [f32; 3],
}

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Owner rotation relative to the connected frame, right-handed about its X axis.
pub struct HingeJoint {
    pub settings: JointSettings,
    pub frame: JointFrame,
    pub connected_frame: JointFrame,
    pub limits: Option<JointLimits>,
    pub motor: Option<JointMotor>,
}

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Owner-anchor displacement relative to the connected anchor along its frame's X axis.
pub struct SliderJoint {
    pub settings: JointSettings,
    pub frame: JointFrame,
    pub connected_frame: JointFrame,
    pub limits: Option<JointLimits>,
    pub motor: Option<JointMotor>,
}

pub(crate) fn validate_limits(limits: &JointLimits, angular: bool) -> Result<()> {
    ensure!(
        limits.min.is_finite()
            && limits.max.is_finite()
            && limits.min <= limits.max
            && (limits.max - limits.min).is_finite(),
        "joint limits must be finite and ordered with a finite span"
    );
    ensure!(
        !angular || limits.max - limits.min <= std::f32::consts::TAU,
        "hinge limit span must not exceed one turn"
    );
    Ok(())
}

pub(crate) fn validate_motor(motor: &JointMotor) -> Result<()> {
    let nonnegative = |v: f32| v.is_finite() && v >= 0.;
    match *motor {
        JointMotor::Velocity { speed, max_effort } => {
            ensure!(speed.is_finite(), "joint motor speed must be finite");
            ensure!(
                nonnegative(max_effort),
                "invalid joint motor maximum effort"
            );
        }
        JointMotor::Position {
            target,
            frequency_hz,
            damping_ratio,
            max_effort,
        } => {
            ensure!(target.is_finite(), "joint motor target must be finite");
            ensure!(
                frequency_hz.is_finite() && frequency_hz > 0.,
                "joint motor frequency must be finite and positive"
            );
            ensure!(
                nonnegative(damping_ratio),
                "invalid joint motor damping ratio"
            );
            ensure!(
                nonnegative(max_effort),
                "invalid joint motor maximum effort"
            );
            let omega = std::f64::consts::TAU * f64::from(frequency_hz);
            let stiffness = (omega * omega) as f32;
            let damping = (2. * f64::from(damping_ratio) * omega) as f32;
            ensure!(
                stiffness.is_finite() && stiffness > 0. && damping.is_finite(),
                "joint motor response is not representable"
            );
        }
    }
    Ok(())
}

fn attachments(entity: &EntityData) -> impl Iterator<Item = (&JointSettings, [f32; 3], [f32; 3])> {
    [
        entity
            .point_joint
            .as_ref()
            .map(|j| (&j.settings, j.anchor, j.connected_anchor)),
        entity
            .hinge_joint
            .as_ref()
            .map(|j| (&j.settings, j.frame.anchor, j.connected_frame.anchor)),
        entity
            .slider_joint
            .as_ref()
            .map(|j| (&j.settings, j.frame.anchor, j.connected_frame.anchor)),
    ]
    .into_iter()
    .flatten()
}

pub(crate) fn validate(entity: &EntityData) -> Result<()> {
    for (settings, _, _) in attachments(entity) {
        ensure!(
            entity.physics.is_some() && entity.parent.is_none() && entity.character.is_none(),
            "joint owner {} requires a non-character root physics body",
            entity.id
        );
        ensure!(
            !matches!(&settings.connection, JointConnection::Body(EntityRef::Local { entity: id }) if *id == entity.id),
            "joint {} cannot connect a body to itself",
            entity.id
        );
    }
    Ok(())
}

fn scaled_anchor(anchor: [f32; 3], scale: [f32; 3]) -> Result<()> {
    let scaled = std::array::from_fn::<_, 3, _>(|i| anchor[i] * scale[i]);
    ensure!(
        scaled.into_iter().all(f32::is_finite)
            && scaled
                .into_iter()
                .map(|v| f64::from(v).powi(2))
                .sum::<f64>()
                <= f64::from(f32::MAX),
        "joint anchor exceeds supported range"
    );
    Ok(())
}

pub(crate) fn validate_connections(snapshot: &SceneSnapshot) -> Result<()> {
    if snapshot.entities.iter().all(|entity| {
        entity.point_joint.is_none()
            && entity.hinge_joint.is_none()
            && entity.slider_joint.is_none()
    }) {
        return Ok(());
    }
    let entities: BTreeMap<_, _> = snapshot.entities.iter().map(|e| (e.id, e)).collect();
    for entity in &snapshot.entities {
        for (settings, anchor, connected_anchor) in attachments(entity) {
            scaled_anchor(anchor, entity.transform.scale)
                .with_context(|| format!("joint {} owner anchor", entity.id))?;
            let scale = match &settings.connection {
                JointConnection::Body(EntityRef::Local { entity: id }) => {
                    let target = entities
                        .get(id)
                        .context("joint connected body is missing")?;
                    ensure!(
                        target.physics.is_some() && target.character.is_none(),
                        "joint {} target {id} requires a non-character physics body",
                        entity.id
                    );
                    ensure!(
                        !settings.enabled
                            || entity
                                .physics
                                .as_ref()
                                .is_some_and(|body| body.motion == BodyMotion::Dynamic)
                            || target
                                .physics
                                .as_ref()
                                .is_some_and(|body| body.motion == BodyMotion::Dynamic),
                        "active joint {} requires at least one dynamic body",
                        entity.id
                    );
                    target.transform.scale
                }
                JointConnection::World => {
                    ensure!(
                        !settings.enabled
                            || entity
                                .physics
                                .as_ref()
                                .is_some_and(|body| body.motion == BodyMotion::Dynamic),
                        "active world joint {} requires a dynamic owner",
                        entity.id
                    );
                    [1.; 3]
                }
                JointConnection::Unbound
                | JointConnection::Body(
                    EntityRef::External { .. } | EntityRef::InInstance { .. },
                ) => [1.; 3],
            };
            scaled_anchor(connected_anchor, scale)
                .with_context(|| format!("joint {} connected anchor", entity.id))?;
        }
    }
    Ok(())
}
