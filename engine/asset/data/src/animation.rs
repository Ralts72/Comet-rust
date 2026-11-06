//! Immutable rig products use source node identities, never expanded scene identities.
use anyhow::{Result, ensure};
use comet_core::{AssetId, EntityId};
use comet_scene::{MAX_ANIMATOR_BINDINGS, Transform, schema};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const ANIMATION_VERSION: u32 = 1;
pub const ANIMATION_CLIP_VERSION: u32 = 2;
pub const MAX_SKIN_JOINTS: usize = 128;
pub const MAX_ANIMATION_CHANNELS: usize = MAX_ANIMATOR_BINDINGS * 3;
pub const MAX_ANIMATION_KEYS: usize = 262_144;
pub const MAX_ANIMATION_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SKIN_VERTICES: usize = 262_144;
pub type SkeletonLibrary = BTreeMap<AssetId, Arc<SkeletonData>>;
pub type AnimationLibrary = BTreeMap<AssetId, Arc<AnimationClipData>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkeletonNode {
    pub id: EntityId,
    pub name: String,
    pub parent: Option<EntityId>,
    pub rest: Transform,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkeletonData {
    pub version: u32,
    pub nodes: Vec<SkeletonNode>,
}
impl SkeletonData {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == ANIMATION_VERSION,
            "unsupported skeleton version"
        );
        ensure!(
            !self.nodes.is_empty() && self.nodes.len() <= MAX_ANIMATOR_BINDINGS,
            "skeleton node budget exceeded"
        );
        let parents: BTreeMap<_, _> = self.nodes.iter().map(|n| (n.id, n.parent)).collect();
        ensure!(parents.len() == self.nodes.len(), "duplicate skeleton node");
        for node in &self.nodes {
            ensure!(
                !node.name.trim().is_empty() && node.name.len() <= 512 && !node.name.contains('\0'),
                "invalid skeleton node name"
            );
            schema::descriptor(schema::TRANSLATION)?
                .validate_value(&schema::FieldValue::Vec3(node.rest.translation))?;
            schema::descriptor(schema::ROTATION)?
                .validate_value(&schema::FieldValue::Quat(node.rest.rotation))?;
            schema::descriptor(schema::SCALE)?
                .validate_value(&schema::FieldValue::Vec3(node.rest.scale))?;
            let mut seen = BTreeSet::new();
            let mut next = Some(node.id);
            while let Some(id) = next {
                ensure!(seen.insert(id), "skeleton hierarchy cycle");
                next = *parents
                    .get(&id)
                    .ok_or_else(|| anyhow::anyhow!("missing skeleton parent {id}"))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinJoint {
    pub node: EntityId,
    /// Column-major, in the source skin's joint order.
    pub inverse_bind: [[f32; 4]; 4],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinData {
    pub version: u32,
    pub skeleton: AssetId,
    pub joints: Vec<SkinJoint>,
}
impl SkinData {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == ANIMATION_VERSION,
            "unsupported skin version"
        );
        ensure!(
            !self.joints.is_empty() && self.joints.len() <= MAX_SKIN_JOINTS,
            "skin joint budget exceeded"
        );
        let mut nodes = BTreeSet::new();
        for joint in &self.joints {
            ensure!(nodes.insert(joint.node), "duplicate skin joint");
            ensure!(
                joint.inverse_bind.iter().flatten().all(|v| v.is_finite()),
                "non-finite inverse bind matrix"
            );
            ensure!(
                [
                    joint.inverse_bind[0][3],
                    joint.inverse_bind[1][3],
                    joint.inverse_bind[2][3],
                    joint.inverse_bind[3][3]
                ] == [0., 0., 0., 1.],
                "inverse bind matrix must be affine"
            );
        }
        Ok(())
    }
    pub fn validate_for(&self, skeleton: &SkeletonData) -> Result<()> {
        self.validate()?;
        skeleton.validate()?;
        let nodes: BTreeSet<_> = skeleton.nodes.iter().map(|n| n.id).collect();
        ensure!(
            self.joints.iter().all(|j| nodes.contains(&j.node)),
            "skin joint is outside its skeleton"
        );
        Ok(())
    }
    pub fn validate_mesh(&self, mesh: &crate::product::MeshData) -> Result<()> {
        self.validate()?;
        mesh.validate()?;
        let joints = mesh
            .joints
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("skin requires mesh joint attributes"))?;
        ensure!(
            joints
                .iter()
                .flatten()
                .all(|j| usize::from(*j) < self.joints.len()),
            "mesh joint index is outside skin-local table"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationInterpolation {
    Step,
    Linear,
    /// Each key stores its incoming tangent, value, then outgoing tangent.
    CubicSpline,
}
impl AnimationInterpolation {
    pub fn values_per_key(self) -> usize {
        match self {
            Self::Step | Self::Linear => 1,
            Self::CubicSpline => 3,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AnimationValues {
    Translation(Vec<[f32; 3]>),
    Rotation(Vec<[f32; 4]>),
    Scale(Vec<[f32; 3]>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationChannel {
    pub node: EntityId,
    pub interpolation: AnimationInterpolation,
    pub times: Vec<f32>,
    pub values: AnimationValues,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationClipData {
    pub version: u32,
    pub skeleton: AssetId,
    pub duration: f32,
    pub channels: Vec<AnimationChannel>,
}
impl AnimationClipData {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=ANIMATION_CLIP_VERSION).contains(&self.version),
            "unsupported animation clip version"
        );
        ensure!(
            self.duration.is_finite() && self.duration >= 0.,
            "invalid animation duration"
        );
        ensure!(
            !self.channels.is_empty() && self.channels.len() <= MAX_ANIMATION_CHANNELS,
            "animation channel budget exceeded"
        );
        let mut targets = BTreeSet::new();
        let mut total = 0usize;
        let mut duration = 0f32;
        for channel in &self.channels {
            let cubic = channel.interpolation == AnimationInterpolation::CubicSpline;
            ensure!(
                !cubic || (self.version >= 2 && channel.times.len() >= 2),
                "cubic animation requires clip version 2 and at least two keys"
            );
            total = total
                .checked_add(channel.times.len())
                .ok_or_else(|| anyhow::anyhow!("animation key budget overflow"))?;
            ensure!(
                !channel.times.is_empty() && total <= MAX_ANIMATION_KEYS,
                "animation key budget exceeded"
            );
            ensure!(
                channel.times.iter().all(|t| t.is_finite() && *t >= 0.)
                    && channel.times.windows(2).all(|v| v[0] < v[1]),
                "animation times must be finite, nonnegative and strictly increasing"
            );
            let (kind, count) = match &channel.values {
                AnimationValues::Translation(values) | AnimationValues::Scale(values) => {
                    ensure!(
                        values.iter().flatten().all(|v| v.is_finite()),
                        "non-finite animation vector"
                    );
                    (
                        u8::from(matches!(channel.values, AnimationValues::Scale(_))),
                        values.len(),
                    )
                }
                AnimationValues::Rotation(values) => {
                    for (index, value) in values.iter().enumerate() {
                        if cubic && index % 3 != 1 {
                            ensure!(
                                value.iter().all(|v| v.is_finite()),
                                "non-finite animation rotation tangent"
                            );
                            continue;
                        }
                        schema::descriptor(schema::ROTATION)?
                            .validate_value(&schema::FieldValue::Quat(*value))?;
                    }
                    (2, values.len())
                }
            };
            ensure!(
                count == channel.times.len() * channel.interpolation.values_per_key(),
                "animation time/value count mismatch"
            );
            ensure!(
                targets.insert((channel.node, kind)),
                "duplicate animation target property"
            );
            duration = duration.max(*channel.times.last().unwrap());
        }
        ensure!(
            duration == self.duration,
            "animation duration does not match keys"
        );
        Ok(())
    }
    pub fn validate_for(&self, skeleton: &SkeletonData) -> Result<()> {
        self.validate()?;
        skeleton.validate()?;
        let nodes: BTreeSet<_> = skeleton.nodes.iter().map(|n| n.id).collect();
        ensure!(
            self.channels.iter().all(|c| nodes.contains(&c.node)),
            "animation target is outside its skeleton"
        );
        Ok(())
    }
}
