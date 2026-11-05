use super::Document;
use anyhow::{Context, Result, ensure};
use comet_core::{EntityId, math::Mat4};
use comet_scene::{PrefabResolver, Transform, checked_transform_inverse, world_transforms};
use std::collections::{BTreeMap, BTreeSet};

impl Document {
    /// Move an authored subtree without changing its world pose or its attached prefab identities.
    /// Prefab interiors stay owned by their source; a non-TRS result is rejected instead of distorted.
    pub fn reparent_entity(
        &mut self,
        id: EntityId,
        parent: Option<EntityId>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.reparent_entities(&[id], parent, resolver)
    }

    /// Reparent selected top-level authored roots as one command. Duplicates and selected
    /// descendants move only once; an empty selection leaves content and history unchanged.
    pub fn reparent_entities(
        &mut self,
        ids: &[EntityId],
        parent: Option<EntityId>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.change(resolver, |next| {
            let parents: BTreeMap<_, _> = next
                .base
                .entities
                .iter()
                .map(|entity| (entity.id, entity.parent))
                .collect();
            let selected: BTreeSet<_> = ids.iter().copied().collect();
            for id in &selected {
                ensure!(
                    parents.contains_key(id),
                    "only authored objects can be reparented"
                );
            }
            let roots: Vec<_> = selected
                .iter()
                .copied()
                .filter(|id| {
                    let mut ancestor = parents[id];
                    while let Some(id) = ancestor {
                        if selected.contains(&id) {
                            return false;
                        }
                        ancestor = parents[&id];
                    }
                    parents[id] != parent
                })
                .collect();
            if roots.is_empty() {
                return Ok(());
            }
            let transforms = world_transforms(&next.base)?;
            let parent_world = parent
                .map(|id| {
                    transforms
                        .get(&id)
                        .copied()
                        .context("parent must be an authored object in this document")
                })
                .transpose()?
                .unwrap_or(Mat4::IDENTITY);
            let inverse = checked_transform_inverse(parent_world)?;
            for id in roots {
                // Validate cycles and physics-root constraints only on the private candidate.
                next.base.reparent(id, parent)?;
                let world = transforms[&id];
                let transform = Transform::from_matrix(inverse * world)?;
                ensure!(
                    matrices_match(parent_world * transform.matrix(), world),
                    "cannot preserve world transform at this parent"
                );
                next.base
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == id)
                    .expect("candidate retains selected object")
                    .transform = transform;
            }
            Ok(())
        })
    }
}

fn matrices_match(left: Mat4, right: Mat4) -> bool {
    left.to_cols_array()
        .into_iter()
        .zip(right.to_cols_array())
        .all(|(a, b)| a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-5 * b.abs().max(1.))
}
