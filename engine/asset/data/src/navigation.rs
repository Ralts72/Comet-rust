//! Offline navigation geometry and freshness are independent of stores and navigation libraries.
use anyhow::{Result, ensure};
use comet_core::validate_digest;
use comet_scene::SceneSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
mod input;
pub use input::{NavigationInput, NavigationProfile};

pub const NAVIGATION_VERSION: u32 = 1;
pub const MAX_NAVIGATION_VERTICES: usize = u16::MAX as usize;
pub const MAX_NAVIGATION_POLYGONS: usize = u16::MAX as usize;
pub const MAX_NAVIGATION_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationData {
    pub version: u32,
    pub input_digest: String,
    /// Comet coordinates: X/right, Y/up, -Z/forward.
    pub vertices: Vec<[f32; 3]>,
    /// Convex polygons in counter-clockwise XZ order, using vertex indices.
    pub polygons: Vec<Vec<u32>>,
}
impl NavigationData {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == NAVIGATION_VERSION,
            "unsupported navigation version"
        );
        validate_digest(&self.input_digest)?;
        ensure!(
            (3..=MAX_NAVIGATION_VERTICES).contains(&self.vertices.len())
                && (1..=MAX_NAVIGATION_POLYGONS).contains(&self.polygons.len()),
            "navigation mesh exceeds its vertex or polygon budget, or is empty"
        );
        ensure!(
            self.vertices
                .iter()
                .flatten()
                .all(|value| value.is_finite()),
            "nonfinite navigation vertex"
        );
        let mut edges = BTreeMap::new();
        for (index, polygon) in self.polygons.iter().enumerate() {
            ensure!(
                (3..=6).contains(&polygon.len()),
                "navigation polygon {index} needs 3 to 6 vertices"
            );
            let mut unique = BTreeSet::new();
            for &vertex in polygon {
                ensure!(
                    (vertex as usize) < self.vertices.len() && unique.insert(vertex),
                    "invalid or repeated vertex in navigation polygon {index}"
                );
            }
            let mut area = 0_f64;
            let origin = self.vertices[polygon[0] as usize];
            for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
                let va = self.vertices[*a as usize];
                let vb = self.vertices[*b as usize];
                ensure!(
                    va[0] != vb[0] || va[2] != vb[2],
                    "collapsed navigation edge"
                );
                area += (f64::from(va[0]) - f64::from(origin[0]))
                    * (f64::from(vb[2]) - f64::from(origin[2]))
                    - (f64::from(vb[0]) - f64::from(origin[0]))
                        * (f64::from(va[2]) - f64::from(origin[2]));
                for &c in polygon {
                    let vc = self.vertices[c as usize];
                    let cross = (f64::from(vb[0]) - f64::from(va[0]))
                        * (f64::from(vc[2]) - f64::from(va[2]))
                        - (f64::from(vb[2]) - f64::from(va[2]))
                            * (f64::from(vc[0]) - f64::from(va[0]));
                    ensure!(
                        cross >= 0.,
                        "navigation polygon {index} is concave or has clockwise XZ winding"
                    );
                }
                let key = ((*a).min(*b), (*a).max(*b));
                match edges.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert((*a, *b, false));
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let (previous_a, previous_b, joined) = entry.get_mut();
                        ensure!(
                            !*joined && *previous_a == *b && *previous_b == *a,
                            "navigation edge has inconsistent or nonmanifold connectivity"
                        );
                        *joined = true;
                    }
                }
            }
            ensure!(
                area > 0. && area.is_finite(),
                "navigation polygon {index} has no projected area"
            );
        }
        Ok(())
    }
    pub fn validate_for(&self, snapshot: &SceneSnapshot) -> Result<()> {
        let input = NavigationInput::capture(snapshot)?;
        self.validate_input(&input)
    }
    /// Reuse a previously captured immutable input, e.g. an editor's cached freshness query.
    pub fn validate_input(&self, input: &NavigationInput) -> Result<()> {
        self.validate()?;
        ensure!(
            self.input_digest == input.digest(),
            "navigation data is stale; bake the region again"
        );
        let bounds = input.bounds();
        ensure!(
            self.vertices.iter().all(|vertex| (0..3).all(|axis| {
                let cell = if axis == 1 {
                    input.profile().cell_height
                } else {
                    input.profile().cell_size
                };
                vertex[axis] >= bounds[0][axis] - cell && vertex[axis] <= bounds[1][axis] + cell
            })),
            "navigation mesh is outside its baking region"
        );
        Ok(())
    }
}
