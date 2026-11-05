//! CPU geometry queries consume the exact immutable batch submitted to the renderer.
use crate::{FramePacket, PickFeedback, PickRequest};
use comet_asset_data::product::Product;
use comet_core::{
    EntityId,
    math::{DVec3, Vec3},
};
use std::collections::BTreeSet;

pub(crate) fn pick(frame: &FramePacket) -> Option<PickFeedback> {
    let request = frame.pick?;
    let entities = if frame.viewport[2] == 0 || frame.viewport[3] == 0 {
        vec![]
    } else {
        match request {
            PickRequest::Point(uv) => point(frame, uv).into_iter().collect(),
            PickRequest::Rect { min, max } => rectangle(frame, min, max),
        }
    };
    Some(PickFeedback {
        token: frame.token,
        entities,
    })
}

fn point(frame: &FramePacket, uv: [f32; 2]) -> Option<EntityId> {
    let aspect = frame.viewport[2] as f32 / frame.viewport[3] as f32;
    let inverse = frame.camera.view_projection(aspect).inverse();
    let far = inverse.project_point3(Vec3::new(uv[0] * 2. - 1., 1. - uv[1] * 2., 1.));
    let origin = frame.camera.eye.as_dvec3();
    let direction = (far.as_dvec3() - origin).try_normalize()?;
    let mut closest = f64::INFINITY;
    let mut entity = None;
    for draw in frame.draws.iter() {
        let Product::Mesh(mesh) = &draw.mesh.payload else {
            continue;
        };
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c]: [DVec3; 3] = triangle.map(|i| {
                match &draw.skin {
                    Some(skin) => skin.position(mesh, i as usize),
                    None => draw
                        .transform
                        .model()
                        .transform_point3(Vec3::from_array(mesh.positions[i as usize])),
                }
                .as_dvec3()
            });
            if !a.is_finite() || !b.is_finite() || !c.is_finite() {
                continue;
            }
            let ab = b - a;
            let ac = c - a;
            let normal = ab.cross(ac);
            let determinant = -direction.dot(normal);
            // Dot-product roundoff scales with area, without a minimum world-space area.
            if determinant.abs() <= 8. * f64::EPSILON * normal.length() {
                continue;
            }
            let p = direction.cross(ac);
            let inv = 1. / determinant;
            let from = origin - a;
            let u = from.dot(p) * inv;
            let q = from.cross(ab);
            let v = direction.dot(q) * inv;
            let t = ac.dot(q) * inv;
            if u >= 0. && v >= 0. && u + v <= 1. && t > 0. && t < closest {
                closest = t;
                entity = Some(draw.entity);
            }
        }
    }
    entity
}

fn rectangle(frame: &FramePacket, min: [f32; 2], max: [f32; 2]) -> Vec<EntityId> {
    if min[0] == max[0] || min[1] == max[1] {
        return vec![];
    }
    let left = f64::from(min[0]) * 2. - 1.;
    let right = f64::from(max[0]) * 2. - 1.;
    let bottom = 1. - f64::from(max[1]) * 2.;
    let top = 1. - f64::from(min[1]) * 2.;
    let planes = [
        [1., 0., 0., -left],
        [-1., 0., 0., right],
        [0., 1., 0., -bottom],
        [0., -1., 0., top],
        [0., 0., 1., 0.],
        [0., 0., -1., 1.],
    ];
    let vp = frame
        .camera
        .view_projection(frame.viewport[2] as f32 / frame.viewport[3] as f32)
        .to_cols_array_2d()
        .map(|column| column.map(f64::from));
    let mut selected = BTreeSet::new();
    let mut polygon = Vec::with_capacity(10);
    let mut clipped = Vec::with_capacity(10);
    for draw in frame.draws.iter() {
        if selected.contains(&draw.entity) {
            continue;
        }
        let Product::Mesh(mesh) = &draw.mesh.payload else {
            continue;
        };
        let mvp = draw
            .transform
            .model()
            .to_cols_array_2d()
            .map(|column| transform(&vp, column.map(f64::from)));
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let vertices = triangle.map(|i| {
                match &draw.skin {
                    Some(skin) => skin.position(mesh, i as usize),
                    None => Vec3::from_array(mesh.positions[i as usize]),
                }
                .as_dvec3()
            });
            // Static models are invertible; skinned triangles may collapse at an animated pose.
            if (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]) == DVec3::ZERO {
                continue;
            }
            polygon.clear();
            polygon
                .extend(vertices.map(|vertex| transform(&mvp, [vertex.x, vertex.y, vertex.z, 1.])));
            if polygon.iter().flatten().any(|value| !value.is_finite()) {
                continue;
            }
            for plane in planes {
                clip(&polygon, &mut clipped, plane);
                std::mem::swap(&mut polygon, &mut clipped);
                if polygon.is_empty() {
                    break;
                }
            }
            // The depth planes imply w >= 0. The homogeneous origin alone has no visible projection.
            if polygon.iter().any(|vertex| vertex[3] > 0.) {
                selected.insert(draw.entity);
                break;
            }
        }
    }
    selected.into_iter().collect()
}

fn transform(matrix: &[[f64; 4]; 4], vertex: [f64; 4]) -> [f64; 4] {
    std::array::from_fn(|row| {
        (0..4)
            .map(|column| matrix[column][row] * vertex[column])
            .sum()
    })
}

fn clip(input: &[[f64; 4]], output: &mut Vec<[f64; 4]>, plane: [f64; 4]) {
    output.clear();
    let Some(&last) = input.last() else {
        return;
    };
    let distance =
        |vertex: [f64; 4]| -> f64 { vertex.into_iter().zip(plane).map(|(v, p)| v * p).sum() };
    let mut previous = last;
    let mut previous_distance = distance(previous);
    for &current in input {
        let current_distance = distance(current);
        let inside = current_distance >= 0.;
        if inside != (previous_distance >= 0.) {
            let t = previous_distance / (previous_distance - current_distance);
            output.push(std::array::from_fn(|i| {
                previous[i] * (1. - t) + current[i] * t
            }));
        }
        if inside {
            output.push(current);
        }
        previous = current;
        previous_distance = current_distance;
    }
}
