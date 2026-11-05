use super::*;
use crate::pick;
use comet_asset_data::product::{MaterialData, MeshData, PRODUCT_VERSION, Product, ProductFile};
use comet_core::AssetId;
mod rectangle;
mod skin;
fn triangle() -> Draw {
    let mesh = Arc::new(ProductFile {
        version: PRODUCT_VERSION,
        id: AssetId::new(),
        source_digest: "0".repeat(64),
        payload: Product::Mesh(MeshData {
            version: 1,
            positions: vec![[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]],
            normals: vec![[0., 0., 1.]; 3],
            uvs: vec![[0.; 2]; 3],
            indices: vec![0, 1, 2],
            joints: None,
            weights: None,
        }),
    });
    let material = Arc::new(ProductFile {
        version: PRODUCT_VERSION,
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
    Draw {
        entity: EntityId::new(),
        transform: DrawTransform::IDENTITY,
        skin: None,
        mesh,
        material,
        texture: None,
        mesh_revision: "revision".into(),
        texture_revision: None,
        roughness: 0.5,
        selected: false,
    }
}
fn packet(draws: Arc<[Draw]>) -> FramePacket {
    FramePacket {
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
        draws,
        ui: UiFrame::default(),
        pick: Some(PickRequest::Point([0.5, 0.5])),
        capture: None,
        shadows: true,
    }
}

#[test]
fn triangle_pick_uses_world_transforms_and_returns_the_nearest_entity() {
    let far = triangle();
    let mut near = far.clone();
    near.entity = EntityId::new();
    near.transform = DrawTransform::prepare(Mat4::from_translation(Vec3::Z))
        .unwrap()
        .unwrap();
    let expected = near.entity;
    let mut frame = packet(Arc::from([far, near]));
    frame.validate().unwrap();
    assert_eq!(pick(&frame).unwrap().entities, vec![expected]);
    frame.pick = Some(PickRequest::Point([0., 0.]));
    assert_eq!(pick(&frame).unwrap().entities, vec![]);
    frame.viewport[2] = 65;
    assert!(frame.validate().is_err());
    frame.viewport[2] = 64;
    for scale in [1e-20, 1e-3, 1e20, -1e20] {
        Arc::make_mut(&mut frame.draws)[0].transform =
            DrawTransform::prepare(Mat4::from_scale(Vec3::splat(scale)))
                .unwrap()
                .unwrap();
        frame
            .validate()
            .expect("a prepared transform must not face another absolute determinant threshold");
    }
}

#[test]
fn retained_batches_pick_their_original_geometry_and_keep_old_products_alive() {
    let first = packet(Arc::from([triangle()]));
    let old_mesh = Arc::downgrade(&first.draws[0].mesh);
    let mut next = packet(first.draws.clone());
    let updated = &mut Arc::make_mut(&mut next.draws)[0];
    updated.mesh_revision = "updated".into();
    let Product::Mesh(mesh) = &mut Arc::make_mut(&mut updated.mesh).payload else {
        unreachable!("triangle fixture uses a mesh");
    };
    for position in &mut mesh.positions {
        position[0] += 10.;
    }
    first.validate().unwrap();
    next.validate().unwrap();
    let old_hit = pick(&first).unwrap();
    assert_eq!(old_hit.entities, vec![first.draws[0].entity]);
    assert_eq!(old_hit.token, first.token);
    let new_hit = pick(&next).unwrap();
    assert_eq!(new_hit.entities, vec![]);
    assert_eq!(new_hit.token, next.token);
    assert!(old_mesh.upgrade().is_some());
    drop(first);
    assert!(old_mesh.upgrade().is_none());
    assert_eq!(pick(&next).unwrap().entities, vec![]);
}

#[test]
fn a_visible_small_triangle_near_the_camera_remains_pickable() {
    let mut draw = triangle();
    draw.transform = DrawTransform::prepare(Mat4::from_scale(Vec3::splat(1.5e-4)))
        .unwrap()
        .unwrap();
    let expected = draw.entity;
    let mut frame = packet(Arc::from([draw]));
    frame.size = [1024, 1024];
    frame.viewport = [0, 0, 1024, 1024];
    frame.camera.eye = Vec3::new(0., 0., 0.06);
    frame.validate().unwrap();
    let projection = frame.camera.view_projection(1.);
    let bottom = projection.project_point3(Vec3::new(0., -1.5e-4, 0.));
    let top = projection.project_point3(Vec3::new(0., 1.5e-4, 0.));
    let pixel_height = (top.y - bottom.y) * 0.5 * frame.viewport[3] as f32;
    assert!(pixel_height > 6. && pixel_height < 7.);
    assert!((0. ..=1.).contains(&top.z));
    assert_eq!(pick(&frame).unwrap().entities, vec![expected]);
}

#[test]
fn picking_supports_tiny_huge_and_mirrored_triangles_from_both_sides() {
    for scale in [1e-20, 1.5e-4, 1., 1e20] {
        for mirrored in [-1., 1.] {
            let mut draw = triangle();
            draw.transform =
                DrawTransform::prepare(Mat4::from_scale(Vec3::new(mirrored * scale, scale, scale)))
                    .unwrap()
                    .unwrap();
            let expected = draw.entity;
            let mut frame = packet(Arc::from([draw]));
            for side in [-1., 1.] {
                frame.camera.eye = Vec3::new(0., 0., side * 4.);
                frame.validate().unwrap();
                let hit = pick(&frame).unwrap();
                assert_eq!(
                    hit.entities,
                    vec![expected],
                    "scale {scale}, mirror {mirrored}, camera side {side}"
                );
                assert_eq!(hit.token, frame.token);
            }
        }
    }
}

#[test]
fn picking_orders_nearly_coincident_surfaces_independently_of_submission_order() {
    let far = triangle();
    let mut near = far.clone();
    near.entity = EntityId::new();
    near.transform = DrawTransform::prepare(Mat4::from_translation(Vec3::new(0., 0., 1e-8)))
        .unwrap()
        .unwrap();
    let expected = near.entity;
    for draws in [[far.clone(), near.clone()], [near.clone(), far.clone()]] {
        let frame = packet(Arc::from(draws));
        frame.validate().unwrap();
        assert_eq!(pick(&frame).unwrap().entities, vec![expected]);
    }
}

fn with_positions(positions: [[f32; 3]; 3]) -> Draw {
    let mut draw = triangle();
    let Product::Mesh(mesh) = &mut Arc::make_mut(&mut draw.mesh).payload else {
        unreachable!()
    };
    mesh.positions = positions.into();
    draw
}

#[test]
fn parallel_and_degenerate_triangles_do_not_produce_hits() {
    for positions in [
        [[0., -1., -1.], [0., 1., -1.], [0., 0., 1.]],
        [[-1., 0., 0.], [0., 0., 0.], [1., 0., 0.]],
        [[0.; 3], [0.; 3], [0.; 3]],
    ] {
        let frame = packet(Arc::from([with_positions(positions)]));
        frame.validate().unwrap();
        assert_eq!(pick(&frame).unwrap().entities, vec![]);
    }
}

#[test]
fn triangle_edges_and_vertices_are_inclusive_without_expanding_outside_the_triangle() {
    for positions in [
        [[-1., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
    ] {
        let draw = with_positions(positions);
        let expected = draw.entity;
        let frame = packet(Arc::from([draw]));
        assert_eq!(pick(&frame).unwrap().entities, vec![expected]);
    }
    let draw = with_positions([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]);
    let mut outside = packet(Arc::from([draw]));
    outside.camera.eye = Vec3::new(-1e-4, 0.5, 4.);
    outside.camera.target = Vec3::new(-1e-4, 0.5, 0.);
    outside.validate().unwrap();
    assert_eq!(pick(&outside).unwrap().entities, vec![]);
}
