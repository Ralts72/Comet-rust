use super::*;

#[test]
fn point_and_rectangle_follow_each_retained_pose_instead_of_the_bind_mesh() {
    let mut draw = triangle();
    let mesh = Arc::make_mut(&mut draw.mesh);
    let Product::Mesh(mesh) = &mut mesh.payload else {
        unreachable!()
    };
    mesh.version = 2;
    mesh.joints = Some(vec![[0; 4]; 3]);
    mesh.weights = Some(vec![[1., 0., 0., 0.]; 3]);
    draw.skin = Some(Arc::new(
        crate::SkinPalette::prepare(&draw.mesh, vec![Mat4::IDENTITY]).unwrap(),
    ));
    let mut first = packet(Arc::from([draw]));
    let mut second = packet(first.draws.clone());
    let updated = &mut Arc::make_mut(&mut second.draws)[0];
    updated.skin = Some(Arc::new(
        crate::SkinPalette::prepare(
            &updated.mesh,
            vec![Mat4::from_translation(Vec3::new(8., 0., 0.))],
        )
        .unwrap(),
    ));
    for request in [
        PickRequest::Point([0.5, 0.5]),
        PickRequest::Rect {
            min: [0.4, 0.4],
            max: [0.6, 0.6],
        },
    ] {
        first.pick = Some(request);
        second.pick = Some(request);
        first.validate().unwrap();
        second.validate().unwrap();
        assert_eq!(pick(&first).unwrap().entities, vec![first.draws[0].entity]);
        assert!(pick(&second).unwrap().entities.is_empty());
    }
    let old = Arc::downgrade(first.draws[0].skin.as_ref().unwrap());
    drop(first);
    assert!(old.upgrade().is_none());
    assert_eq!(
        second.draws[0].skin.as_ref().unwrap().matrices()[0]
            .w_axis
            .x,
        8.
    );
    Arc::make_mut(&mut second.draws)[0].transform =
        DrawTransform::prepare(Mat4::from_translation(Vec3::X))
            .unwrap()
            .unwrap();
    assert!(second.validate().is_err());
}

#[test]
#[ignore = "native offscreen skinning, normals and shadow pixel comparison; run explicitly"]
fn native_skinning_matches_static_geometry_normals_and_shadows() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use comet_core::math::Quat;
    use std::{fs, io::BufReader, path::Path};
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/verification");
    fs::create_dir_all(&output)?;
    let report = output.join("render-skinning.json");
    fs::write(&report, "{\"status\":\"running\"}")?;
    let mut worker = crate::RenderWorker::spawn(None)?;
    let mut number = 0;
    let mut capture = |label: &str, draws: Vec<Draw>, shadows: bool| -> anyhow::Result<_> {
        let path = output.join(format!("render-skinning-{label}.png"));
        let mut frame = packet(draws.into());
        frame.number = number;
        frame.size = [160, 160];
        frame.viewport = [0, 0, 160, 160];
        frame.camera.eye = Vec3::new(0., 2., 5.);
        frame.camera.target = Vec3::ZERO;
        frame.capture = Some(path.clone());
        frame.pick = None;
        frame.shadows = shadows;
        worker.submit(frame)?;
        let feedback = worker.wait(number)?;
        number += 1;
        ensure!(feedback.draw_count == 2, "offscreen draw count changed");
        let mut reader = png::Decoder::new(BufReader::new(fs::File::open(&path)?)).read_info()?;
        let mut bytes = vec![0; reader.output_buffer_size().context("PNG dimensions")?];
        let image = reader.next_frame(&mut bytes)?;
        ensure!(
            image.width == 160 && image.height == 160 && image.color_type == png::ColorType::Rgba,
            "unexpected capture format"
        );
        bytes.truncate(image.buffer_size());
        Ok((bytes, feedback, path))
    };
    let mut floor = triangle();
    let floor_mesh = Arc::make_mut(&mut floor.mesh);
    floor_mesh.id = AssetId::new();
    let Product::Mesh(mesh) = &mut floor_mesh.payload else {
        unreachable!()
    };
    mesh.positions = vec![
        [-5., -1.4, -5.],
        [5., -1.4, -5.],
        [5., -1.4, 5.],
        [-5., -1.4, 5.],
    ];
    mesh.normals = vec![[0., 1., 0.]; 4];
    mesh.uvs = vec![[0.; 2]; 4];
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    let mut comparisons = vec![];
    for (label, mirror) in [
        ("regular", false),
        ("mirrored", true),
        ("singular", false),
        ("extreme-nonuniform", false),
    ] {
        let mut skinned = triangle();
        let product = Arc::make_mut(&mut skinned.mesh);
        let Product::Mesh(mesh) = &mut product.payload else {
            unreachable!()
        };
        mesh.version = 2;
        mesh.positions[2][2] = 0.8;
        mesh.normals
            .fill(Vec3::new(0., -0.4, 1.).normalize().to_array());
        mesh.joints = Some(vec![[0, 1, 0, 0]; 3]);
        mesh.weights = Some(vec![[0.25, 0.75, 0., 0.]; 3]);
        let singular = label == "singular";
        let extreme = label == "extreme-nonuniform";
        if extreme {
            let Product::Mesh(mesh) = &mut Arc::make_mut(&mut skinned.mesh).payload else {
                unreachable!()
            };
            for position in &mut mesh.positions {
                position[0] *= 1e25;
                position[1] *= 1e25;
                position[2] = 0.;
            }
            mesh.normals.fill([0., 0., 1.]);
        }
        if singular {
            let Product::Mesh(mesh) = &mut Arc::make_mut(&mut skinned.mesh).payload else {
                unreachable!()
            };
            mesh.weights = Some(vec![[0.5, 0.5, 0., 0.]; 3]);
        }
        let (a, b) = if singular {
            (Mat4::IDENTITY, Mat4::from_scale(Vec3::new(1., 1., -1.)))
        } else if extreme {
            let matrix = Mat4::from_scale(Vec3::new(1e-25, 1e-25, 1.));
            (matrix, matrix)
        } else {
            let a = Mat4::from_scale_rotation_translation(
                Vec3::new(if mirror { -0.7 } else { 0.7 }, 1.1, 0.9),
                Quat::from_rotation_y(0.35) * Quat::from_rotation_x(0.25),
                Vec3::ZERO,
            );
            (a, a * Mat4::from_translation(Vec3::new(0.5, 0., 0.)))
        };
        let mut reference = skinned.clone();
        if singular {
            // The visible XY geometry survives the rank-two blend; the agreed normal is zero.
            let Product::Mesh(mesh) = &mut Arc::make_mut(&mut reference.mesh).payload else {
                unreachable!()
            };
            mesh.normals.fill([0.; 3]);
            for position in &mut mesh.positions {
                position[2] = 0.;
            }
            reference.mesh_revision = "singular-zero-normal-reference".into();
        } else {
            reference.transform = DrawTransform::prepare(a * 0.25 + b * 0.75)?
                .context("reference model collapsed")?;
        }
        skinned.skin = Some(Arc::new(crate::SkinPalette::prepare(
            &skinned.mesh,
            vec![a, b],
        )?));
        let (expected, _, reference_path) = capture(
            &format!("{label}-static"),
            vec![reference, floor.clone()],
            true,
        )?;
        let (actual, feedback, skin_path) =
            capture(label, vec![skinned.clone(), floor.clone()], true)?;
        let total_error: u64 = actual
            .iter()
            .zip(&expected)
            .map(|(&a, &b)| u64::from(a.abs_diff(b)))
            .sum();
        let divergent = actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected.as_chunks::<4>().0)
            .filter(|(a, b)| a.iter().zip(*b).any(|(&a, &b)| a.abs_diff(b) > 4))
            .count();
        let mean_error = total_error as f64 / actual.len() as f64;
        ensure!(
            mean_error <= 0.25 && divergent <= 64,
            "{label} skin/static pixels differ: mean {mean_error}, pixels {divergent}"
        );
        let shadow_pixels = if label == "regular" {
            let (unshadowed, _, _) = capture("no-shadows", vec![skinned, floor.clone()], false)?;
            let changed = actual
                .as_chunks::<4>()
                .0
                .iter()
                .zip(unshadowed.as_chunks::<4>().0)
                .filter(|(a, b)| a.iter().zip(*b).any(|(&a, &b)| a.abs_diff(b) > 4))
                .count();
            ensure!(
                changed > 8,
                "shadow comparison did not exercise a visible cast shadow"
            );
            Some(changed)
        } else {
            None
        };
        comparisons.push(serde_json::json!({"case":label,"backend":feedback.backend,"mean_channel_error":mean_error,"divergent_pixels":divergent,"shadow_pixels":shadow_pixels,"reference":reference_path,"capture":skin_path}));
    }
    worker.close()?;
    fs::write(
        report,
        serde_json::to_vec_pretty(
            &serde_json::json!({"status":"passed","frames":number,"comparisons":comparisons}),
        )?,
    )?;
    Ok(())
}
