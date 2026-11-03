use super::*;
use comet_asset_data::product::{MaterialData, MeshData, Product, ProductFile};
use comet_core::AssetId;
#[test]
fn triangle_pick_uses_world_transforms_and_returns_the_nearest_entity() {
    let mesh = Arc::new(ProductFile {
        version: 1,
        id: AssetId::new(),
        source_digest: "0".repeat(64),
        payload: Product::Mesh(MeshData {
            version: 1,
            positions: vec![[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]],
            normals: vec![[0., 0., 1.]; 3],
            uvs: vec![[0.; 2]; 3],
            indices: vec![0, 1, 2],
        }),
    });
    let material = Arc::new(ProductFile {
        version: 1,
        id: AssetId::new(),
        source_digest: "0".repeat(64),
        payload: Product::Material(MaterialData {
            version: 1,
            base_color: [1.; 4],
            roughness: 0.5,
            metallic: 0.,
            base_color_texture: None,
        }),
    });
    let far = Draw {
        entity: EntityId::new(),
        transform: Mat4::IDENTITY,
        mesh,
        material,
        texture: None,
        mesh_revision: "revision".into(),
        texture_revision: None,
        roughness: 0.5,
        selected: false,
    };
    let mut near = far.clone();
    near.entity = EntityId::new();
    near.transform = Mat4::from_translation(Vec3::Z);
    let expected = near.entity;
    let mut frame = FramePacket {
        number: 0,
        token: ViewToken {
            world: WorldId::new(),
            view: 1,
            target_generation: 1,
        },
        size: [64, 64],
        viewport: [0, 0, 64, 64],
        camera: Camera {
            eye: Vec3::new(0., 0., 4.),
            target: Vec3::ZERO,
        },
        draws: vec![far, near],
        ui: UiFrame::default(),
        pick: Some([0.5, 0.5]),
        capture: None,
        shadows: true,
    };
    frame.validate().unwrap();
    assert_eq!(pick(&frame).unwrap().entity, Some(expected));
    frame.pick = Some([0., 0.]);
    assert_eq!(pick(&frame).unwrap().entity, None);
    frame.viewport[2] = 65;
    assert!(frame.validate().is_err());
}
