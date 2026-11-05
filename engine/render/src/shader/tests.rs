use super::*;
use comet_asset_data::product::{MaterialData, MeshData, PRODUCT_VERSION, Product, ProductFile};
use comet_core::{AssetId, EntityId, math::Vec3};
use std::sync::Arc;

#[test]
fn shaders_and_layouts_compile_for_both_backends() {
    for ui in [false, true] {
        let compiled = compile(ui).unwrap();
        assert!(!compiled.msl.is_empty());
        if !ui {
            for name in [
                "scene_vs",
                "shadow_vs",
                "skinned_scene_vs",
                "skinned_shadow_vs",
            ] {
                assert!(compiled.names.contains_key(name) && compiled.spirv.contains_key(name));
            }
            assert_eq!(std::mem::size_of::<Influence>(), 32);
            assert_eq!(std::mem::size_of::<Palette>(), 128 * 64);
        }
        assert!(
            compiled
                .spirv
                .values()
                .all(|s| s.first() == Some(&0x07230203))
        );
    }
}

#[test]
fn uploaded_normals_preserve_huge_tiny_and_subnormal_directions_and_leave_zero_unchanged() {
    let mesh = MeshData {
        version: 1,
        positions: vec![[0.; 3]; 4],
        normals: vec![
            [1e38, -2e38, 3e38],
            [1e-30, -2e-30, 3e-30],
            [f32::from_bits(1), -f32::from_bits(2), f32::from_bits(3)],
            [0.; 3],
        ],
        uvs: vec![[0.; 2]; 4],
        indices: vec![0, 1, 2],
        joints: None,
        weights: None,
    };
    mesh.validate().unwrap();
    let uploaded = vertices(&mesh);
    let expected = Vec3::new(1., -2., 3.).normalize();
    for vertex in &uploaded[..3] {
        let normal = Vec3::new(vertex[3], vertex[4], vertex[5]);
        assert!(normal.is_finite());
        assert!((normal - expected).length() < 1e-6);
    }
    assert_eq!(&uploaded[3][3..6], &[0.; 3]);
}

#[test]
fn object_uniforms_reuse_the_prepared_normal_for_a_huge_model() -> Result<()> {
    let product = |payload| {
        Arc::new(ProductFile {
            version: PRODUCT_VERSION,
            id: AssetId::new(),
            source_digest: "0".repeat(64),
            payload,
        })
    };
    let model = Mat4::from_scale_rotation_translation(
        Vec3::splat(1e20),
        comet_core::math::Quat::IDENTITY,
        Vec3::splat(1e35),
    );
    let draw = crate::Draw {
        entity: EntityId::new(),
        transform: crate::DrawTransform::prepare(model)?.unwrap(),
        skin: None,
        mesh: product(Product::Mesh(MeshData {
            version: 1,
            positions: vec![[0.; 3]; 3],
            normals: vec![[0., 1., 0.]; 3],
            uvs: vec![[0.; 2]; 3],
            indices: vec![0, 1, 2],
            joints: None,
            weights: None,
        })),
        material: product(Product::Material(MaterialData {
            version: 1,
            base_color: [1.; 4],
            roughness: 0.5,
            metallic: 0.,
            base_color_texture: None,
        })),
        texture: None,
        mesh_revision: "revision".into(),
        texture_revision: None,
        roughness: 0.5,
        selected: false,
    };
    let object = Object::for_draw(&draw, true)?;
    assert_eq!(object.model, model.to_cols_array_2d());
    assert_eq!(object.normal, Mat4::IDENTITY.to_cols_array_2d());
    Ok(())
}
