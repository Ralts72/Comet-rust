//! A position transform and its GPU-safe normal matrix are prepared together before submission.
use anyhow::{Context, Result, ensure};
use comet_core::math::{DMat3, Mat3, Mat4};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawTransform {
    model: Mat4,
    normal: Mat3,
}

impl DrawTransform {
    pub const IDENTITY: Self = Self {
        model: Mat4::IDENTITY,
        normal: Mat3::IDENTITY,
    };

    /// Collapsed geometry has no usable normal transform and is omitted from both render passes.
    /// Other inputs must preserve a common positive inverse-transpose scale at GPU precision.
    pub fn prepare(model: Mat4) -> Result<Option<Self>> {
        ensure!(model.is_finite(), "non-finite draw transform");
        ensure!(
            model.x_axis.w == 0.
                && model.y_axis.w == 0.
                && model.z_axis.w == 0.
                && model.w_axis.w == 1.,
            "draw transform must be affine"
        );
        let linear = DMat3::from_cols(
            model.x_axis.truncate().as_dvec3(),
            model.y_axis.truncate().as_dvec3(),
            model.z_axis.truncate().as_dvec3(),
        );
        let Some(reference) = normal_reference(linear)? else {
            return Ok(None);
        };
        let mut coefficients = reference.as_mat3().to_cols_array();
        for value in &mut coefficients {
            *value = gpu_coefficient(*value);
        }
        let normal = Mat3::from_cols_array(&coefficients);
        // Check the pair GPU arithmetic can use, while preserving the original model for positions.
        let effective_linear =
            Mat3::from_cols_array(&linear.as_mat3().to_cols_array().map(gpu_coefficient))
                .as_dmat3();
        let effective_reference = if effective_linear == linear {
            reference
        } else {
            normal_reference(effective_linear)?
                .context("model transform loses an axis at GPU precision")?
        };
        ensure!(
            matching_columns(effective_reference, reference)
                && matching_columns(normal.as_dmat3(), reference),
            "normal transform cannot be represented reliably at GPU precision"
        );
        Ok(Some(Self { model, normal }))
    }

    pub fn model(&self) -> Mat4 {
        self.model
    }

    pub fn normal(&self) -> Mat3 {
        self.normal
    }
}

fn normal_reference(linear: DMat3) -> Result<Option<DMat3>> {
    // Products of finite f32 linear coefficients fit in f64, independent of object size.
    let cofactors = DMat3::from_cols(
        linear.y_axis.cross(linear.z_axis),
        linear.z_axis.cross(linear.x_axis),
        linear.x_axis.cross(linear.y_axis),
    );
    let determinant = linear.x_axis.dot(cofactors.x_axis);
    if determinant == 0. {
        return Ok(None);
    }
    let inverse_transpose = cofactors / determinant;
    ensure!(
        inverse_transpose.is_finite(),
        "normal transform cannot be inverted reliably"
    );
    let largest = inverse_transpose
        .to_cols_array()
        .into_iter()
        .map(f64::abs)
        .fold(0_f64, f64::max);
    ensure!(largest > 0., "normal transform has no direction");
    Ok(Some(inverse_transpose / largest))
}

fn matching_columns(actual: DMat3, reference: DMat3) -> bool {
    // A dual-product residual amplifies harmless f32 rounding by the model's condition number.
    // Compare directions and relative column magnitudes without an absolute size floor instead.
    actual.is_finite()
        && [actual.x_axis, actual.y_axis, actual.z_axis]
            .into_iter()
            .zip([reference.x_axis, reference.y_axis, reference.z_axis])
            .all(|(actual, reference)| {
                let scale = reference.abs().max_element();
                let error = (actual - reference).abs().max_element() / scale;
                scale > 0. && error.is_finite() && error <= 1e-5 && actual.dot(reference) > 0.
            })
}

fn gpu_coefficient(value: f32) -> f32 {
    // GPU arithmetic may flush subnormal values; this is not an authoring-format restriction.
    if value.is_subnormal() { 0. } else { value }
}

#[cfg(test)]
mod tests;
