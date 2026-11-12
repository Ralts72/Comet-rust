use super::*;

fn rectangle(draws: Vec<Draw>, min: [f32; 2], max: [f32; 2]) -> FramePacket {
    let mut frame = packet(draws.into());
    frame.pick = Some(PickRequest::Rect { min, max });
    frame
}

fn hits(frame: &FramePacket) -> Vec<EntityId> {
    frame.validate().unwrap();
    let feedback = pick(frame).unwrap();
    assert_eq!(feedback.token, frame.token);
    feedback.entities
}

#[test]
fn rectangle_intersects_triangles_instead_of_origins_vertices_or_bounds() {
    let covering = with_positions([[-4., -4., 0.], [4., -4., 0.], [0., 4., 0.]]);
    let expected = covering.entity;
    let frame = rectangle(vec![covering], [0.45; 2], [0.55; 2]);
    let Product::Mesh(mesh) = &frame.draws[0].mesh.payload else {
        unreachable!()
    };
    for position in &mesh.positions {
        let p = frame
            .camera
            .view_projection(1.)
            .project_point3(Vec3::from_array(*position));
        let uv = [p.x * 0.5 + 0.5, 0.5 - p.y * 0.5];
        assert!((0..2).any(|axis| uv[axis] < 0.45 || uv[axis] > 0.55));
    }
    assert_eq!(hits(&frame), [expected]);

    let mut offset = with_positions([[9., -1., 0.], [11., -1., 0.], [10., 1., 0.]]);
    offset.transform = DrawTransform::prepare(Mat4::from_translation(Vec3::new(-10., 0., 0.)))
        .unwrap()
        .unwrap();
    let expected = offset.entity;
    assert_eq!(
        hits(&rectangle(vec![offset], [0.45; 2], [0.55; 2])),
        [expected]
    );
    let outside = with_positions([[9., -1., 0.], [11., -1., 0.], [10., 1., 0.]]);
    assert!(hits(&rectangle(vec![outside], [0.45; 2], [0.55; 2])).is_empty());

    let corner = with_positions([[0., 1., 0.], [1., 1., 0.], [1., 0., 0.]]);
    assert!(hits(&rectangle(vec![corner], [0.51, 0.47], [0.53, 0.49])).is_empty());
}

#[test]
fn rectangle_clips_the_visible_triangle_before_any_perspective_division() {
    for (positions, visible) in [
        ([[-0.5, -0.5, -1.], [0.5, -0.5, -1.], [0., 0.5, -1.]], true),
        ([[-0.5, -0.5, 1.], [0.5, -0.5, 1.], [0., 0.5, 1.]], false),
        (
            [[-0.5, -0.5, -0.025], [0.5, -0.5, -0.025], [0., 0.5, -0.025]],
            false,
        ),
        (
            [[-1., -1., -201.], [1., -1., -201.], [0., 1., -201.]],
            false,
        ),
        ([[-0.5, -0.5, -1.], [0.5, -0.5, -1.], [0., 0.5, 0.1]], true),
        ([[-1., -1., -199.], [1., -1., -199.], [0., 1., -201.]], true),
        ([[10., -0.5, -1.], [11., -0.5, -1.], [10., 0.5, -1.]], false),
    ] {
        let draw = with_positions(positions);
        let expected = draw.entity;
        let mut frame = rectangle(vec![draw], [0.; 2], [1.; 2]);
        frame.camera = Camera {
            eye: Vec3::ZERO,
            target: -Vec3::Z,
            ..Default::default()
        };
        assert_eq!(
            hits(&frame),
            if visible { vec![expected] } else { vec![] },
            "{positions:?}"
        );
    }
}

#[test]
fn rectangle_returns_occluded_entities_once_in_stable_order_while_point_stays_nearest() {
    let mut far = triangle();
    far.entity = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let mut near = far.clone();
    near.entity = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    near.transform = DrawTransform::prepare(Mat4::from_translation(Vec3::Z))
        .unwrap()
        .unwrap();
    for draws in [
        vec![near.clone(), far.clone(), far.clone()],
        vec![far.clone(), far.clone(), near.clone()],
    ] {
        let mut frame = rectangle(draws, [0.45; 2], [0.55; 2]);
        assert_eq!(hits(&frame), [far.entity, near.entity]);
        frame.pick = Some(PickRequest::Point([0.5; 2]));
        assert_eq!(hits(&frame), [near.entity]);
    }
    // Point and rectangle queries both respect the rendered depth range.
    let mut frame = rectangle(vec![far], [0.; 2], [1.; 2]);
    Arc::make_mut(&mut frame.draws)[0].transform =
        DrawTransform::prepare(Mat4::from_translation(Vec3::new(0., 0., -300.)))
            .unwrap()
            .unwrap();
    assert!(hits(&frame).is_empty());
    frame.pick = Some(PickRequest::Point([0.5; 2]));
    assert!(hits(&frame).is_empty());
}

#[test]
fn rectangle_supports_extreme_scales_mirrors_both_sides_and_closed_boundaries() {
    for scale in [1e-20, 1.5e-4, 1., 1e20] {
        for mirrored in [-1., 1.] {
            let mut draw = triangle();
            draw.transform =
                DrawTransform::prepare(Mat4::from_scale(Vec3::new(mirrored * scale, scale, scale)))
                    .unwrap()
                    .unwrap();
            let expected = draw.entity;
            let mut frame = rectangle(vec![draw], [0.49; 2], [0.51; 2]);
            for side in [-1., 1.] {
                frame.camera.eye = Vec3::new(0., 0., side * 4.);
                assert_eq!(
                    hits(&frame),
                    [expected],
                    "scale {scale}, mirror {mirrored}, side {side}"
                );
            }
        }
    }
    for positions in [
        [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        [[0., -1., 0.], [1., -1., 0.], [0., 1., 0.]],
    ] {
        let draw = with_positions(positions);
        let expected = draw.entity;
        let mut frame = rectangle(vec![draw], [0.4, 0.5], [0.5, 0.6]);
        assert_eq!(hits(&frame), [expected], "boundary {positions:?}");
        frame.pick = Some(PickRequest::Rect {
            min: [0.4, 0.5],
            max: [0.5 - 1e-6, 0.6],
        });
        assert!(hits(&frame).is_empty(), "outside {positions:?}");
    }
    for positions in [[[0.; 3]; 3], [[-1., 0., 0.], [0., 0., 0.], [1., 0., 0.]]] {
        assert!(
            hits(&rectangle(
                vec![with_positions(positions)],
                [0.; 2],
                [1.; 2]
            ))
            .is_empty()
        );
    }
    let edge_on = with_positions([[0., -1., -1.], [0., 1., -1.], [0., 0., 1.]]);
    let expected = edge_on.entity;
    assert_eq!(
        hits(&rectangle(vec![edge_on], [0.49; 2], [0.51; 2])),
        [expected]
    );
}

#[test]
fn retained_rectangle_batches_keep_their_own_geometry_products_and_token() {
    let first = rectangle(vec![triangle()], [0.45; 2], [0.55; 2]);
    let old_mesh = Arc::downgrade(&first.draws[0].mesh);
    let mut next = rectangle(vec![], [0.45; 2], [0.55; 2]);
    next.draws = first.draws.clone();
    let draw = &mut Arc::make_mut(&mut next.draws)[0];
    let Product::Mesh(mesh) = &mut Arc::make_mut(&mut draw.mesh).payload else {
        unreachable!()
    };
    for position in &mut mesh.positions {
        position[0] += 10.;
    }
    assert_eq!(hits(&first), [first.draws[0].entity]);
    assert!(hits(&next).is_empty());
    assert!(old_mesh.upgrade().is_some());
    drop(first);
    assert!(old_mesh.upgrade().is_none());
    assert!(hits(&next).is_empty());
}

#[test]
fn pick_coordinates_are_validated_and_empty_rectangles_or_viewports_report_no_hits() {
    let mut frame = rectangle(vec![triangle()], [0.; 2], [1.; 2]);
    for request in [
        PickRequest::Point([f32::NAN, 0.5]),
        PickRequest::Point([0.5, f32::INFINITY]),
        PickRequest::Point([-0.1, 0.5]),
        PickRequest::Point([0.5, 1.1]),
        PickRequest::Rect {
            min: [0.6, 0.4],
            max: [0.5, 0.6],
        },
        PickRequest::Rect {
            min: [0.4, 0.6],
            max: [0.6, 0.5],
        },
        PickRequest::Rect {
            min: [0.; 2],
            max: [1.1, 1.],
        },
        PickRequest::Rect {
            min: [f32::NEG_INFINITY, 0.],
            max: [1.; 2],
        },
    ] {
        frame.pick = Some(request);
        assert!(frame.validate().is_err(), "{request:?}");
    }
    for (min, max) in [([0.5; 2], [0.5, 0.6]), ([0.5; 2], [0.6, 0.5])] {
        frame.pick = Some(PickRequest::Rect { min, max });
        assert!(hits(&frame).is_empty());
    }
    for request in [
        PickRequest::Point([0.5; 2]),
        PickRequest::Rect {
            min: [0.; 2],
            max: [1.; 2],
        },
    ] {
        frame.pick = Some(request);
        for viewport in [[0, 0, 0, 64], [0, 0, 64, 0]] {
            frame.viewport = viewport;
            assert!(hits(&frame).is_empty());
        }
    }
    frame.pick = None;
    assert!(pick(&frame).is_none());
}
