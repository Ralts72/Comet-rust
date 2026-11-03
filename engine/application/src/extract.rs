use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::EntityId;
use comet_core::math::{Mat4, Quat, Vec3};
use comet_render::Draw;
use comet_scene::SceneSnapshot;
use std::collections::BTreeMap;

pub fn world_transforms(snapshot: &SceneSnapshot) -> Result<BTreeMap<EntityId, Mat4>> {
    snapshot.validate()?;
    let mut result = BTreeMap::new();
    let mut pending: Vec<_> = snapshot.entities.iter().collect();
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|e| {
            let parent = match e.parent {
                Some(id) => match result.get(&id) {
                    Some(m) => *m,
                    None => return true,
                },
                None => Mat4::IDENTITY,
            };
            result.insert(
                e.id,
                parent
                    * Mat4::from_scale_rotation_translation(
                        Vec3::from_array(e.transform.scale),
                        Quat::from_array(e.transform.rotation),
                        Vec3::from_array(e.transform.translation),
                    ),
            );
            false
        });
        ensure!(pending.len() < before, "unresolved transform hierarchy");
    }
    Ok(result)
}
pub fn extract(
    snapshot: &SceneSnapshot,
    store: &mut AssetStore,
    selected: Option<EntityId>,
) -> Result<Vec<Draw>> {
    let transforms = world_transforms(snapshot)?;
    let mut draws = vec![];
    for e in &snapshot.entities {
        if let Some(model) = &e.model {
            let transform = transforms[&e.id];
            if transform.determinant().abs() < 1e-8 {
                continue;
            }
            let mesh = store.load(model.mesh)?;
            ensure!(
                matches!(mesh.payload, Product::Mesh(_)),
                "entity {} has a non-mesh reference",
                e.id
            );
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
            draws.push(Draw {
                entity: e.id,
                transform,
                mesh,
                material,
                texture,
                mesh_revision,
                texture_revision,
                roughness: model.surface.roughness,
                selected: selected == Some(e.id),
            });
        }
    }
    Ok(draws)
}
