use super::*;
use comet_asset_data::product::PRODUCT_VERSION;
use comet_core::AssetId;

fn mesh() -> Arc<ProductFile> {
    Arc::new(ProductFile {
        version: PRODUCT_VERSION,
        id: AssetId::new(),
        source_digest: "0".repeat(64),
        payload: Product::Mesh(MeshData {
            version: 2,
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            normals: vec![[0., 0., 1.]; 3],
            uvs: vec![[0.; 2]; 3],
            indices: vec![0, 1, 2],
            joints: Some(vec![[0, 1, 0, 0]; 3]),
            weights: Some(vec![[0.25, 0.75, 0., 0.]; 3]),
        }),
    })
}

#[test]
fn weighted_world_poses_are_immutable_and_collapsed_blends_remain_legal() -> Result<()> {
    let product = mesh();
    let Product::Mesh(data) = &product.payload else {
        unreachable!()
    };
    let first = SkinPalette::prepare(
        &product,
        vec![
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(4., 0., 0.)),
        ],
    )?;
    assert_eq!(first.position(data, 0), Vec3::new(3., 0., 0.));
    let next = SkinPalette::prepare(
        &product,
        vec![Mat4::IDENTITY, Mat4::from_scale(Vec3::splat(-1. / 3.))],
    )?;
    assert!(next.position(data, 1).length() < 1e-7);
    assert_eq!(first.position(data, 1), Vec3::new(4., 0., 0.));
    assert!(first.matches_mesh(&product));
    assert!(!first.matches_mesh(&Arc::new((*product).clone())));
    Ok(())
}

#[test]
fn invalid_palettes_and_gpu_overflow_fail_before_publication() -> Result<()> {
    let product = mesh();
    assert!(SkinPalette::prepare(&product, vec![]).is_err());
    assert!(SkinPalette::prepare(&product, vec![Mat4::IDENTITY]).is_err());
    assert!(SkinPalette::prepare(&product, vec![Mat4::IDENTITY; MAX_SKIN_JOINTS + 1]).is_err());
    SkinPalette::prepare(&product, vec![Mat4::IDENTITY; MAX_SKIN_JOINTS])?;
    for matrix in [
        Mat4::from_cols_array(&[f32::NAN; 16]),
        comet_core::math::perspective(1., 1., 0.1, 10.),
    ] {
        assert!(SkinPalette::prepare(&product, vec![matrix; 2]).is_err());
    }
    let mut huge = (*product).clone();
    let Product::Mesh(data) = &mut huge.payload else {
        unreachable!()
    };
    data.positions[0] = [f32::MAX, f32::MAX, 0.];
    let matrix = Mat4::from_cols_array_2d(&[
        [2., 0., 0., 0.],
        [2., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]);
    assert!(SkinPalette::prepare(&Arc::new(huge), vec![matrix; 2]).is_err());
    Ok(())
}

#[test]
fn extreme_anisotropic_poses_preserve_visible_geometry_and_a_representable_static_normal()
-> Result<()> {
    let mut product = mesh();
    let Product::Mesh(data) = &mut Arc::make_mut(&mut product).payload else {
        unreachable!()
    };
    for position in &mut data.positions {
        position[0] *= 1e25;
        position[1] *= 1e25;
    }
    let matrix = Mat4::from_scale(Vec3::new(1e-25, 1e-25, 1.));
    let skin = SkinPalette::prepare(&product, vec![matrix; 2])?;
    let Product::Mesh(data) = &product.payload else {
        unreachable!()
    };
    assert!((skin.position(data, 1) - Vec3::X).length() < 1e-6);
    assert!((skin.position(data, 2) - Vec3::Y).length() < 1e-6);
    let normal = crate::DrawTransform::prepare(matrix)?
        .expect("visible static reference")
        .normal()
        * Vec3::Z;
    assert!(normal.is_finite());
    assert_eq!(normal.as_dvec3().normalize().as_vec3(), Vec3::Z);
    // Global normalization loses its determinant despite the usable geometry and normal.
    assert_eq!(1e-25_f32 * 1e-25, 0.);
    Ok(())
}
