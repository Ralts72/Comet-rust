use super::*;
use comet_core::math::{Quat, Vec3, Vec4};

#[test]
fn tiny_huge_and_mirrored_uniform_scales_keep_finite_oriented_normals() -> Result<()> {
    for scale in [1e-20_f32, 1e-3, 1e20, -1e20] {
        let model = Mat4::from_scale_rotation_translation(
            Vec3::splat(scale),
            Quat::IDENTITY,
            Vec3::new(1e35, -1e35, 1e35),
        );
        let prepared = DrawTransform::prepare(model)?.expect("nonzero uniform scale is drawable");
        assert_eq!(prepared.model(), model);
        assert!(prepared.normal().is_finite());
        let expected = Mat3::from_diagonal(Vec3::splat(scale.signum()));
        for (actual, expected) in prepared
            .normal()
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!((actual - expected).abs() < 1e-6);
        }
    }
    Ok(())
}

#[test]
fn nonuniform_sheared_and_reflected_normals_remain_dual_to_the_world_tangents() -> Result<()> {
    for model in [
        Mat4::from_cols(
            Vec4::new(-2., 0.25, 0.75, 0.),
            Vec4::new(0.5, 3., 0.2, 0.),
            Vec4::new(0.1, -0.5, 4., 0.),
            Vec4::new(1e35, -1e35, 1e35, 1.),
        ),
        Mat4::from_scale_rotation_translation(
            Vec3::new(-0.002, 3., 5e8),
            Quat::from_rotation_y(0.7),
            Vec3::ZERO,
        ),
    ] {
        let prepared = DrawTransform::prepare(model)?.unwrap();
        let axes = [
            model.x_axis.truncate(),
            model.y_axis.truncate(),
            model.z_axis.truncate(),
        ];
        let normal_axes = [
            prepared.normal().x_axis,
            prepared.normal().y_axis,
            prepared.normal().z_axis,
        ];
        for index in 0..3 {
            let normal = normal_axes[index].as_dvec3().normalize();
            assert!(
                normal.dot(axes[index].as_dvec3()) > 0.,
                "reflection must keep the authored outward side"
            );
            for (tangent, axis) in axes.iter().enumerate() {
                if tangent != index {
                    assert!(normal.dot(axis.as_dvec3().normalize()).abs() < 1e-6);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn only_collapsed_geometry_is_omitted_and_invalid_inputs_are_errors() -> Result<()> {
    assert!(DrawTransform::prepare(Mat4::from_scale(Vec3::new(0., 2., 3.)))?.is_none());
    let dependent = Mat4::from_cols(
        Vec3::X.extend(0.),
        Vec3::Y.extend(0.),
        (Vec3::X + Vec3::Y).extend(0.),
        Vec4::W,
    );
    assert!(DrawTransform::prepare(dependent)?.is_none());
    let mut non_affine = Mat4::IDENTITY;
    non_affine.x_axis.w = 0.1;
    assert!(DrawTransform::prepare(non_affine).is_err());
    let mut non_finite = Mat4::IDENTITY;
    non_finite.w_axis.x = f32::NAN;
    assert!(DrawTransform::prepare(non_finite).is_err());
    Ok(())
}

#[test]
fn lost_gpu_axes_are_rejected_using_each_columns_own_relative_scale() -> Result<()> {
    // After common normalization one required normal coefficient is 1e-40 and gets flushed.
    // A fixed absolute tolerance would accept the missing column because its magnitude is 1e-40.
    assert!(DrawTransform::prepare(Mat4::from_scale(Vec3::new(1e-20, 1e20, 1.))).is_err());
    // A model axis may itself disappear on GPU before normal multiplication starts.
    assert!(DrawTransform::prepare(Mat4::from_scale(Vec3::splat(1e-40))).is_err());
    // An immaterial subnormal shear coefficient need not reject an otherwise usable transform.
    let mut harmless = Mat4::IDENTITY;
    harmless.y_axis.x = 1e-40;
    assert!(DrawTransform::prepare(harmless)?.is_some());
    Ok(())
}
