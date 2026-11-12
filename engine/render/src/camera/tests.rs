use super::*;

#[test]
fn fitted_views_contain_bounds_and_reproject_the_focus_plane_in_both_projections() {
    for aspect in [0.1, 0.5, 1., 2.4, 10.] {
        for projection in [
            Projection::Perspective,
            Projection::Orthographic { height: 4. },
        ] {
            for [min, max] in [
                [Vec3::new(-3., -1., -2.), Vec3::new(7., 5., 4.)],
                [Vec3::splat(-0.0001), Vec3::splat(0.0001)],
                [Vec3::splat(-200.), Vec3::splat(200.)],
                [Vec3::new(3., 2., -7.); 2],
            ] {
                let camera = Camera {
                    projection,
                    ..Default::default()
                }
                .fit_bounds([min, max], aspect)
                .unwrap();
                camera.validate().unwrap();
                let vp = camera.view_projection(aspect);
                for x in [min.x, max.x] {
                    for y in [min.y, max.y] {
                        for z in [min.z, max.z] {
                            let p = vp.project_point3(Vec3::new(x, y, z));
                            assert!(
                                p.x.abs() < 1. && p.y.abs() < 1. && (0. ..=1.).contains(&p.z),
                                "{camera:?}: {p:?}"
                            );
                        }
                    }
                }
                for uv in [[0.5, 0.5], [0., 0.], [1., 1.], [0.2, 0.8]] {
                    let p = camera.focus_plane_point(uv, aspect).unwrap();
                    let clip = vp.project_point3(p);
                    assert!((clip.x - (uv[0] * 2. - 1.)).abs() < 0.002);
                    assert!((clip.y - (1. - uv[1] * 2.)).abs() < 0.002);
                }
            }
        }
    }
    let camera = Camera::default();
    for bounds in [
        [Vec3::ONE, Vec3::ZERO],
        [Vec3::splat(f32::NAN), Vec3::ONE],
        [Vec3::splat(f32::MAX), Vec3::splat(f32::MAX)],
    ] {
        assert!(camera.fit_bounds(bounds, 1.).is_err());
    }
    for aspect in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(camera.fit_bounds([Vec3::ZERO, Vec3::ONE], aspect).is_err());
    }
    for projection in [
        Projection::Orthographic { height: 0. },
        Projection::Orthographic {
            height: f32::INFINITY,
        },
    ] {
        assert!(
            Camera {
                projection,
                ..camera
            }
            .validate()
            .is_err()
        );
    }
}
