use anyhow::{Context, Result, ensure};
use comet_core::{EntityId, digest};
use comet_scene::{BodyMotion, ColliderShape, SceneSnapshot};
const MAX_BOXES: usize = 4096;
const MAX_TRIANGLES: usize = 262_144;
const MAX_GROUND_CELLS: usize = 262_144;
const MAX_VOXELS: usize = 8_388_608;

#[derive(Clone, Debug)]
pub struct NavigationProfile {
    pub cell_size: f32,
    pub cell_height: f32,
    pub agent_radius: f32,
    pub agent_height: f32,
    pub max_climb: f32,
    pub max_slope_degrees: f32,
}

#[derive(Debug)]
pub struct NavigationInput {
    region: EntityId,
    digest: String,
    bounds: [[f32; 3]; 2],
    profile: NavigationProfile,
    triangles: Vec<[[f32; 3]; 3]>,
}
impl NavigationInput {
    pub fn region(&self) -> EntityId {
        self.region
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn bounds(&self) -> [[f32; 3]; 2] {
        self.bounds
    }
    pub fn profile(&self) -> &NavigationProfile {
        &self.profile
    }
    pub fn triangles(&self) -> &[[[f32; 3]; 3]] {
        &self.triangles
    }

    pub fn capture(snapshot: &SceneSnapshot) -> Result<Self> {
        snapshot.validate()?;
        let mut regions = snapshot
            .entities
            .iter()
            .filter(|entity| entity.navigation_region.is_some());
        let region = regions.next().context("navigation requires one region")?;
        ensure!(
            regions.next().is_none(),
            "navigation supports exactly one region"
        );
        let settings = region.navigation_region.as_ref().unwrap();
        let profile = NavigationProfile {
            cell_size: settings.cell_size,
            cell_height: (settings.cell_size * 0.5).min(0.1),
            agent_radius: settings.agent_radius,
            agent_height: settings.agent_height,
            max_climb: settings.max_climb,
            max_slope_degrees: settings.max_slope_degrees,
        };
        let center = region.transform.translation;
        let bounds = [
            std::array::from_fn(|i| zero(center[i] - settings.half_extents[i])),
            std::array::from_fn(|i| zero(center[i] + settings.half_extents[i])),
        ];
        validate_grid(bounds, &profile)?;
        let mut boxes = vec![];
        let mut triangles = vec![];
        for entity in &snapshot.entities {
            let Some(body) = &entity.physics else {
                continue;
            };
            if body.motion != BodyMotion::Fixed || body.sensor {
                continue;
            }
            let position = entity.transform.translation.map(f64::from);
            let axes = normalized_axes(entity.transform.rotation);
            let scale = entity.transform.scale;
            let half = match body.shape {
                ColliderShape::Box { half_extents } => {
                    std::array::from_fn(|i| f64::from(half_extents[i] * scale[i]))
                }
                ColliderShape::Sphere { radius } => [f64::from(radius * scale[0]); 3],
                ColliderShape::Capsule {
                    half_height,
                    radius,
                } => std::array::from_fn(|i| {
                    axes[1][i].abs() * f64::from(half_height * scale[0])
                        + f64::from(radius * scale[0])
                }),
            };
            let aabb_half = if matches!(body.shape, ColliderShape::Box { .. }) {
                std::array::from_fn(|i| (0..3).map(|axis| axes[axis][i].abs() * half[axis]).sum())
            } else {
                half
            };
            if (0..3).any(|i| {
                position[i] + aabb_half[i] < f64::from(bounds[0][i])
                    || position[i] - aabb_half[i] > f64::from(bounds[1][i])
            }) {
                continue;
            }
            ensure!(
                matches!(body.shape, ColliderShape::Box { .. }),
                "navigation region intersects unsupported fixed collider {}; only solid boxes can be baked",
                entity.id
            );
            ensure!(
                boxes.len() < MAX_BOXES,
                "navigation geometry exceeds its box budget"
            );
            let vertices: [[f64; 3]; 8] = std::array::from_fn(|corner| {
                std::array::from_fn(|i| {
                    zero64(
                        position[i]
                            + (0..3)
                                .map(|axis| {
                                    axes[axis][i]
                                        * half[axis]
                                        * if corner & (1 << axis) == 0 { -1. } else { 1. }
                                })
                                .sum::<f64>(),
                    )
                })
            });
            // Clipping enormous edges into a small voxel volume must retain sub-cell precision.
            ensure!(
                vertices.iter().all(|vertex| (0..3).all(|axis| {
                    let cell = if axis == 1 {
                        profile.cell_height
                    } else {
                        profile.cell_size
                    };
                    (vertex[axis] - f64::from(center[axis])).abs()
                        <= f64::from(cell) * 1_073_741_824.
                })),
                "navigation collision geometry exceeds the clipping precision budget"
            );
            let mut key = vertices;
            key.sort_by(compare_vertex);
            boxes.push(key);
            // Outward-facing triangles, including walls and ceilings as unwalkable raster input.
            for [a, b, c] in [
                [0, 2, 1],
                [1, 2, 3],
                [4, 5, 6],
                [5, 7, 6],
                [0, 4, 2],
                [2, 4, 6],
                [1, 3, 5],
                [3, 7, 5],
                [0, 1, 4],
                [1, 5, 4],
                [2, 6, 3],
                [3, 6, 7],
            ] {
                clip_triangle(
                    [vertices[a], vertices[b], vertices[c]],
                    bounds,
                    &mut triangles,
                )?;
            }
        }
        ensure!(
            !triangles.is_empty(),
            "navigation region has no supported static collision geometry"
        );
        boxes.sort_by(|a, b| {
            a.iter()
                .zip(b)
                .map(|(a, b)| compare_vertex(a, b))
                .find(|order| !order.is_eq())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut bytes =
            b"comet-navigation-boxes-v1\0rerecast-0.5.0\0watershed\0max-poly-6\0detail-none\0filter-lowhanging-ledges-clearance\0erode-ceil-radius\0maxedge-12\0error-1.3\0minregion-0\0mergeregion-400\0"
                .to_vec();
        for value in bounds.into_iter().flatten().chain([
            profile.cell_size,
            profile.cell_height,
            profile.agent_radius,
            profile.agent_height,
            profile.max_climb,
            profile.max_slope_degrees,
        ]) {
            bytes.extend_from_slice(&zero(value).to_bits().to_le_bytes());
        }
        bytes.extend_from_slice(&(boxes.len() as u64).to_le_bytes());
        for value in boxes.iter().flatten().flatten() {
            bytes.extend_from_slice(&zero64(*value).to_bits().to_le_bytes());
        }
        triangles.sort_by(|a, b| {
            a.iter()
                .flatten()
                .zip(b.iter().flatten())
                .map(|(a, b)| a.total_cmp(b))
                .find(|order| !order.is_eq())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(Self {
            region: region.id,
            digest: digest(&bytes),
            bounds,
            profile,
            triangles,
        })
    }
}

fn validate_grid(bounds: [[f32; 3]; 2], p: &NavigationProfile) -> Result<()> {
    ensure!(
        bounds.iter().flatten().all(|v| v.is_finite())
            && (0..3).all(|i| bounds[0][i] < bounds[1][i]),
        "navigation bounds are not representable"
    );
    ensure!(
        (0.05..=2.).contains(&p.cell_size)
            && p.agent_radius > 0.
            && p.agent_height >= p.agent_radius * 2.
            && p.agent_height <= 20.
            && p.max_climb >= 0.
            && p.max_climb <= p.agent_height
            && (0. ..90.).contains(&p.max_slope_degrees),
        "navigation profile is invalid"
    );
    let mut counts = [0_usize; 3];
    for i in 0..3 {
        let cell = if i == 1 { p.cell_height } else { p.cell_size };
        ensure!(
            bounds[0][i] + cell > bounds[0][i] && bounds[1][i] - cell < bounds[1][i],
            "navigation cells are too small for these world coordinates"
        );
        let count = ((f64::from(bounds[1][i]) - f64::from(bounds[0][i])) / f64::from(cell)).ceil();
        ensure!(
            count >= if i == 1 { 1. } else { 3. } && count <= if i == 1 { 65534. } else { 2048. },
            "navigation grid dimension exceeds its budget"
        );
        counts[i] = count as usize;
    }
    ensure!(
        counts[0]
            .checked_mul(counts[2])
            .is_some_and(|count| count <= MAX_GROUND_CELLS)
            && counts[0]
                .checked_mul(counts[1])
                .and_then(|count| count.checked_mul(counts[2]))
                .is_some_and(|count| count <= MAX_VOXELS),
        "navigation voxel grid exceeds its cell budget"
    );
    for (value, cell) in [
        (p.agent_height, p.cell_height),
        (p.max_climb, p.cell_height),
        (p.agent_radius, p.cell_size),
    ] {
        ensure!(
            value.is_finite() && (f64::from(value) / f64::from(cell)).ceil() <= u16::MAX as f64,
            "navigation profile exceeds voxel index capacity"
        );
    }
    Ok(())
}
fn zero(value: f32) -> f32 {
    if value == 0. { 0. } else { value }
}
fn zero64(value: f64) -> f64 {
    if value == 0. { 0. } else { value }
}
fn compare_vertex(a: &[f64; 3], b: &[f64; 3]) -> std::cmp::Ordering {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.total_cmp(b))
        .find(|o| !o.is_eq())
        .unwrap_or(std::cmp::Ordering::Equal)
}
fn normalized_axes(rotation: [f32; 4]) -> [[f64; 3]; 3] {
    let mut q = rotation.map(f64::from);
    let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    for v in &mut q {
        *v /= norm;
    }
    let [x, y, z, w] = q;
    [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y + z * w),
            2. * (x * z - y * w),
        ],
        [
            2. * (x * y - z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z + x * w),
        ],
        [
            2. * (x * z + y * w),
            2. * (y * z - x * w),
            1. - 2. * (x * x + y * y),
        ],
    ]
}
fn clip_triangle(
    triangle: [[f64; 3]; 3],
    bounds: [[f32; 3]; 2],
    output: &mut Vec<[[f32; 3]; 3]>,
) -> Result<()> {
    let mut polygon = triangle.to_vec();
    for axis in 0..3 {
        for (plane, sign) in [
            (f64::from(bounds[0][axis]), 1.),
            (f64::from(bounds[1][axis]), -1.),
        ] {
            if polygon.is_empty() {
                return Ok(());
            }
            let mut next = Vec::with_capacity(polygon.len() + 1);
            let mut previous = *polygon.last().unwrap();
            let mut previous_distance = (previous[axis] - plane) * sign;
            for &current in &polygon {
                let distance = (current[axis] - plane) * sign;
                if (distance >= 0.) != (previous_distance >= 0.) {
                    let ratio = previous_distance / (previous_distance - distance);
                    let mut vertex =
                        std::array::from_fn(|i| previous[i] + (current[i] - previous[i]) * ratio);
                    vertex[axis] = plane;
                    next.push(vertex);
                }
                if distance >= 0. {
                    next.push(current);
                }
                previous = current;
                previous_distance = distance;
            }
            polygon = next;
        }
    }
    for i in 1..polygon.len().saturating_sub(1) {
        let vertices = [polygon[0], polygon[i], polygon[i + 1]];
        let a: [f64; 3] = std::array::from_fn(|j| vertices[1][j] - vertices[0][j]);
        let b: [f64; 3] = std::array::from_fn(|j| vertices[2][j] - vertices[0][j]);
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        if cross.iter().all(|v| *v == 0.) {
            continue;
        }
        ensure!(
            output.len() < MAX_TRIANGLES,
            "navigation geometry exceeds its triangle budget"
        );
        output.push(vertices.map(|v| v.map(|v| zero(v as f32))));
    }
    Ok(())
}
