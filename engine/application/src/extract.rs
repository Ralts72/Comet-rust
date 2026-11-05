use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::{Product, ProductIndex};
use comet_core::{EntityId, math::Mat4};
use comet_render::{Draw, DrawTransform, SkinPalette};
use comet_scene::{EntityRef, SceneSnapshot, UnboundRig, world_transforms};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Only the latest successful immutable scene/product version is retained.
#[derive(Default)]
pub(crate) struct SceneExtractor {
    cached: Option<CachedExtraction>,
}

struct CachedExtraction {
    snapshot: Arc<SceneSnapshot>,
    index: Arc<ProductIndex>,
    selected: Vec<EntityId>,
    draws: Arc<[Draw]>,
}

impl SceneExtractor {
    pub(crate) fn extract(
        &mut self,
        snapshot: &Arc<SceneSnapshot>,
        store: &mut AssetStore,
        selected: &[EntityId],
    ) -> Result<Arc<[Draw]>> {
        let index = store.shared_index();
        if let Some(cached) = &mut self.cached
            && Arc::ptr_eq(&cached.snapshot, snapshot)
            && Arc::ptr_eq(&cached.index, &index)
        {
            if cached.selected != selected {
                let highlighted = selected_descendants(snapshot, selected);
                let draws: Arc<[Draw]> = cached
                    .draws
                    .iter()
                    .map(|draw| {
                        let mut draw = draw.clone();
                        draw.selected = highlighted.contains(&draw.entity);
                        draw
                    })
                    .collect();
                cached.selected = selected.to_vec();
                cached.draws = draws;
            }
            return Ok(cached.draws.clone());
        }
        // Validation and asset resolution must finish before replacing the accepted result.
        let draws: Arc<[Draw]> = extract(snapshot, store, selected)?.into();
        self.cached = Some(CachedExtraction {
            snapshot: snapshot.clone(),
            index,
            selected: selected.to_vec(),
            draws: draws.clone(),
        });
        Ok(draws)
    }
}

pub fn extract(
    snapshot: &SceneSnapshot,
    store: &mut AssetStore,
    selected: &[EntityId],
) -> Result<Vec<Draw>> {
    let transforms = world_transforms(snapshot)?;
    let highlighted = selected_descendants(snapshot, selected);
    let mut draws = vec![];
    let mut unbound = vec![];
    let entities: HashMap<_, _> = snapshot
        .entities
        .iter()
        .map(|entity| (entity.id, entity))
        .collect();
    for e in &snapshot.entities {
        ensure!(
            comet_scene::schema::entity_references(e)
                .iter()
                .all(|reference| !matches!(reference, EntityRef::InInstance { .. })),
            "entity {} contains an unresolved prefab instance reference",
            e.id
        );
        if let Some(model) = &e.model {
            let Some(transform) = DrawTransform::prepare(if e.skin.is_some() {
                Mat4::IDENTITY
            } else {
                transforms[&e.id]
            })
            .with_context(|| format!("entity {} render transform", e.id))?
            else {
                continue;
            };
            let mesh = store.load(model.mesh)?;
            let Product::Mesh(mesh_data) = &mesh.payload else {
                anyhow::bail!("entity {} has a non-mesh reference", e.id)
            };
            let material = store.load(model.surface.material)?;
            let Product::Material(m) = &material.payload else {
                anyhow::bail!("entity {} has a non-material reference", e.id)
            };
            let texture = m.base_color_texture.map(|id| store.load(id)).transpose()?;
            ensure!(
                texture
                    .as_ref()
                    .is_none_or(|t| matches!(t.payload, Product::Texture(_))),
                "entity {} has a non-texture reference",
                e.id
            );
            let texture_revision = texture
                .as_ref()
                .map(|t| store.index().products[&t.id].digest.clone());
            let mesh_revision = store
                .index()
                .products
                .get(&mesh.id)
                .context("mesh revision")?
                .digest
                .clone();
            let skin = if let Some(binding) = &e.skin {
                let product = store.load(binding.skin)?;
                let Product::Skin(skin) = &product.payload else {
                    anyhow::bail!("entity {} has a non-skin reference", e.id)
                };
                let skeleton_product = store.load(skin.skeleton)?;
                let Product::Skeleton(skeleton) = &skeleton_product.payload else {
                    anyhow::bail!("skin has a non-skeleton reference")
                };
                skin.validate_for(skeleton)?;
                skin.validate_mesh(mesh_data)?;
                let animator = match &binding.animator {
                    None => None,
                    Some(EntityRef::External { .. } | EntityRef::InInstance { .. }) => {
                        anyhow::bail!("skin animator must belong to the same scene")
                    }
                    Some(EntityRef::Local { entity }) => Some(
                        entities
                            .get(entity)
                            .context("skin animator entity is missing")?
                            .animator
                            .as_ref()
                            .context("skin target has no Animator")?,
                    ),
                };
                if let Some(animator) = animator {
                    ensure!(
                        animator.skeleton == skin.skeleton,
                        "skin and animator use different skeletons"
                    );
                    let mut missing = false;
                    // Do not let a missing binding conceal an invalid reference elsewhere in the rig.
                    for node in &skeleton.nodes {
                        match animator.bindings.get(&node.id).and_then(Option::as_ref) {
                            None => missing = true,
                            Some(EntityRef::External { .. } | EntityRef::InInstance { .. }) => {
                                anyhow::bail!("rig bone must belong to the same scene")
                            }
                            Some(EntityRef::Local { entity }) => ensure!(
                                transforms.contains_key(entity),
                                "rig bone entity is missing"
                            ),
                        }
                    }
                    if missing {
                        unbound.push(e.id);
                        None
                    } else {
                        let matrices = skin
                            .joints
                            .iter()
                            .map(|joint| {
                                let Some(EntityRef::Local { entity }) =
                                    animator.bindings[&joint.node].as_ref()
                                else {
                                    unreachable!("complete local rig checked above")
                                };
                                transforms[entity] * Mat4::from_cols_array_2d(&joint.inverse_bind)
                            })
                            .collect();
                        Some(Arc::new(
                            SkinPalette::prepare(&mesh, matrices)
                                .with_context(|| format!("entity {} skin pose", e.id))?,
                        ))
                    }
                } else {
                    unbound.push(e.id);
                    None
                }
            } else {
                None
            };
            if e.skin.is_some() && skin.is_none() {
                continue;
            }
            draws.push(Draw {
                entity: e.id,
                transform,
                skin,
                mesh,
                material,
                texture,
                mesh_revision,
                texture_revision,
                roughness: model.surface.roughness,
                selected: highlighted.contains(&e.id),
            });
        }
    }
    if !unbound.is_empty() {
        return Err(UnboundRig { entities: unbound }.into());
    }
    Ok(draws)
}

fn selected_descendants(snapshot: &SceneSnapshot, selected: &[EntityId]) -> HashSet<EntityId> {
    let mut highlighted: HashSet<_> = selected.iter().copied().collect();
    if !highlighted.is_empty() {
        let mut children: HashMap<EntityId, Vec<EntityId>> = HashMap::new();
        for entity in &snapshot.entities {
            if let Some(parent) = entity.parent {
                children.entry(parent).or_default().push(entity.id);
            }
        }
        let mut pending: Vec<_> = highlighted.iter().copied().collect();
        while let Some(parent) = pending.pop() {
            if let Some(children) = children.get(&parent) {
                for &child in children {
                    if highlighted.insert(child) {
                        pending.push(child);
                    }
                }
            }
        }
    }
    highlighted
}

#[cfg(test)]
mod tests;
