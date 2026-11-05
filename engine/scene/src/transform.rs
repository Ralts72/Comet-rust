use crate::{PoseUpdate, SceneSnapshot, Transform};
use anyhow::{Context, Result, ensure};
use comet_core::{
    EntityId,
    math::{Mat4, Quat, Vec3},
};
use std::collections::BTreeMap;

impl Transform {
    /// Accept only finite, invertible affine TRS matrices; never approximate shear.
    pub fn from_matrix(matrix: Mat4) -> Result<Self> {
        checked_transform_inverse(matrix)?;
        let axes = [
            matrix.x_axis.truncate().normalize(),
            matrix.y_axis.truncate().normalize(),
            matrix.z_axis.truncate().normalize(),
        ];
        ensure!(
            axes[0].dot(axes[1]).abs() < 1e-5
                && axes[0].dot(axes[2]).abs() < 1e-5
                && axes[1].dot(axes[2]).abs() < 1e-5,
            "local transform requires shear; world pose cannot be preserved"
        );
        let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
        let transform = Self {
            scale: scale.to_array(),
            rotation: rotation.normalize().to_array(),
            translation: translation.to_array(),
        };
        ensure!(
            matrices_match(transform.matrix(), matrix),
            "matrix is not a finite affine TRS transform"
        );
        Ok(transform)
    }

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::from_array(self.scale),
            Quat::from_array(self.rotation),
            Vec3::from_array(self.translation),
        )
    }
}

/// Invert a transform only when its axes and inverse are numerically usable.
pub fn checked_transform_inverse(matrix: Mat4) -> Result<Mat4> {
    ensure!(
        matrix.x_axis.w == 0.
            && matrix.y_axis.w == 0.
            && matrix.z_axis.w == 0.
            && matrix.w_axis.w == 1.,
        "transform must be affine"
    );
    let axes = [
        matrix.x_axis.truncate(),
        matrix.y_axis.truncate(),
        matrix.z_axis.truncate(),
    ];
    ensure!(
        matrix.is_finite() && axes.iter().all(|axis| axis.length() > 1e-6),
        "transform is singular or too small"
    );
    let volume = axes[0]
        .normalize()
        .cross(axes[1].normalize())
        .dot(axes[2].normalize());
    ensure!(volume.abs() > 1e-6, "transform is nearly singular");
    let mut inverse = matrix.inverse();
    // The input is affine; canonicalize roundoff from the general matrix inverse.
    inverse.x_axis.w = 0.;
    inverse.y_axis.w = 0.;
    inverse.z_axis.w = 0.;
    inverse.w_axis.w = 1.;
    ensure!(
        inverse.is_finite() && matrices_match(inverse * matrix, Mat4::IDENTITY),
        "transform cannot be inverted reliably"
    );
    Ok(inverse)
}

fn matrices_match(left: Mat4, right: Mat4) -> bool {
    left.to_cols_array()
        .into_iter()
        .zip(right.to_cols_array())
        .all(|(a, b)| a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-5 * b.abs().max(1.))
}

/// Resolve authored or expanded content independently of asset loading, editing and rendering.
pub fn world_transforms(snapshot: &SceneSnapshot) -> Result<BTreeMap<EntityId, Mat4>> {
    snapshot.validate_structure()?;
    let nodes: Vec<_> = snapshot
        .entities
        .iter()
        .map(|entity| TransformNode {
            id: entity.id,
            parent: entity.parent,
            transform: &entity.transform,
        })
        .collect();
    compose_world_transforms(&nodes, &BTreeMap::new())
}

pub(crate) struct TransformNode<'a> {
    pub id: EntityId,
    pub parent: Option<EntityId>,
    pub transform: &'a Transform,
}

/// Structure and pose overrides are validated by the caller before composition.
/// Borrow only transforms so fixed-step validation never copies script parameters or asset data.
pub(crate) fn compose_world_transforms(
    nodes: &[TransformNode<'_>],
    poses: &BTreeMap<EntityId, &PoseUpdate>,
) -> Result<BTreeMap<EntityId, Mat4>> {
    let mut result = BTreeMap::new();
    let by_id: BTreeMap<_, _> = nodes.iter().map(|node| (node.id, node)).collect();
    let mut chain = Vec::new();
    for node in nodes {
        let mut next = Some(node);
        while let Some(node) = next {
            if result.contains_key(&node.id) {
                break;
            }
            chain.push(node);
            ensure!(chain.len() <= nodes.len(), "unresolved transform hierarchy");
            next = node
                .parent
                .map(|parent| {
                    by_id
                        .get(&parent)
                        .copied()
                        .context("missing transform parent")
                })
                .transpose()?;
        }
        while let Some(node) = chain.pop() {
            let parent = node.parent.map_or(Mat4::IDENTITY, |id| result[&id]);
            let local = match poses.get(&node.id) {
                Some(pose) => Mat4::from_scale_rotation_translation(
                    Vec3::from_array(node.transform.scale),
                    Quat::from_array(pose.rotation),
                    Vec3::from_array(pose.translation),
                ),
                None => node.transform.matrix(),
            };
            let world = parent * local;
            ensure!(
                world.is_finite(),
                "world transform overflow for entity {}",
                node.id
            );
            result.insert(node.id, world);
        }
    }
    Ok(result)
}
