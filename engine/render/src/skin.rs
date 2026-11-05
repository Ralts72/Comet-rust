use anyhow::{Result, ensure};
use comet_asset_data::{
    MAX_SKIN_JOINTS,
    product::{MeshData, Product, ProductFile},
};
use comet_core::math::{Mat4, Vec3};
use std::sync::Arc;

/// An immutable, mesh-checked world-space pose. Submitted frames retain their own pose.
#[derive(Debug)]
pub struct SkinPalette {
    mesh: Arc<ProductFile>,
    matrices: Box<[Mat4]>,
}

impl SkinPalette {
    pub fn prepare(mesh: &Arc<ProductFile>, matrices: Vec<Mat4>) -> Result<Self> {
        ensure!(
            !matrices.is_empty() && matrices.len() <= MAX_SKIN_JOINTS,
            "invalid skin palette size"
        );
        let Product::Mesh(data) = &mesh.payload else {
            anyhow::bail!("skin uses a non-mesh product")
        };
        data.validate()?;
        let (Some(joints), Some(weights)) = (&data.joints, &data.weights) else {
            anyhow::bail!("skinned mesh has no influences")
        };
        for matrix in &matrices {
            ensure!(
                matrix.is_finite()
                    && matrix.x_axis.w == 0.
                    && matrix.y_axis.w == 0.
                    && matrix.z_axis.w == 0.
                    && matrix.w_axis.w == 1.,
                "skin palette is not finite affine"
            );
        }
        for ((position, joints), weights) in data.positions.iter().zip(joints).zip(weights) {
            ensure!(
                joints
                    .iter()
                    .all(|&joint| usize::from(joint) < matrices.len()),
                "mesh joint index exceeds skin palette"
            );
            let matrix = blend(&matrices, *joints, *weights);
            ensure!(
                matrix.is_finite(),
                "skin blend exceeds GPU arithmetic range"
            );
            // Check intermediate products too: cancellation must not conceal an overflow on GPU.
            let p = Vec3::from_array(*position).extend(1.);
            let columns = [matrix.x_axis, matrix.y_axis, matrix.z_axis, matrix.w_axis];
            for row in 0..3 {
                let magnitude: f64 = columns
                    .iter()
                    .zip(p.to_array())
                    .map(|(c, v)| (f64::from(c[row]) * f64::from(v)).abs())
                    .sum();
                ensure!(
                    magnitude <= f64::from(f32::MAX),
                    "skinned vertex exceeds GPU arithmetic range"
                );
            }
            ensure!((matrix * p).is_finite(), "skinned vertex is not finite");
        }
        Ok(Self {
            mesh: mesh.clone(),
            matrices: matrices.into_boxed_slice(),
        })
    }

    pub fn matrices(&self) -> &[Mat4] {
        &self.matrices
    }

    pub(crate) fn matches_mesh(&self, mesh: &Arc<ProductFile>) -> bool {
        Arc::ptr_eq(&self.mesh, mesh)
    }

    pub(crate) fn position(&self, mesh: &MeshData, index: usize) -> Vec3 {
        let joints = mesh.joints.as_ref().expect("prepared skin influences")[index];
        let weights = mesh.weights.as_ref().expect("prepared skin weights")[index];
        (blend(&self.matrices, joints, weights)
            * Vec3::from_array(mesh.positions[index]).extend(1.))
        .truncate()
    }
}

fn blend(matrices: &[Mat4], joints: [u16; 4], weights: [f32; 4]) -> Mat4 {
    matrices[usize::from(joints[0])] * weights[0]
        + matrices[usize::from(joints[1])] * weights[1]
        + matrices[usize::from(joints[2])] * weights[2]
        + matrices[usize::from(joints[3])] * weights[3]
}

#[cfg(test)]
mod tests;
