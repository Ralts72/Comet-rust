//! Model commands validate only their target's draw data; unrelated authoring errors remain repairable.
use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::{AssetId, EntityId};
use comet_editor_core::{FieldEdit, FieldTarget};
use comet_scene::{
    Model, Surface,
    schema::{self, FieldKey, FieldValue},
};

fn validate_products(model: &Model, assets: &mut AssetStore) -> Result<()> {
    schema::descriptor(schema::ROUGHNESS)?
        .validate_value(&FieldValue::Scalar(model.surface.roughness))?;
    ensure!(
        matches!(assets.load(model.mesh)?.payload, Product::Mesh(_)),
        "请选择网格产品"
    );
    let material = assets.load(model.surface.material)?;
    let Product::Material(material) = &material.payload else {
        anyhow::bail!("请选择材质产品");
    };
    if let Some(texture) = material.base_color_texture {
        ensure!(
            matches!(assets.load(texture)?.payload, Product::Texture(_)),
            "材质颜色纹理引用无效"
        );
    }
    Ok(())
}

impl Editor {
    pub(crate) fn add_model(
        &mut self,
        id: EntityId,
        mesh: AssetId,
        material: AssetId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let source = self.document.shared_source();
        let entity = source
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .context("只能为作者对象添加模型")?;
        ensure!(entity.model.is_none(), "对象已有模型");
        let product = assets.load(material)?;
        let Product::Material(surface) = &product.payload else {
            anyhow::bail!("请选择材质产品");
        };
        let model = Model {
            mesh,
            surface: Surface {
                material,
                roughness: surface.roughness,
            },
        };
        self.edit_model(id, Some(model), assets)
    }

    pub(crate) fn edit_model(
        &mut self,
        id: EntityId,
        model: Option<Model>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let target = self.resolve_field_target(id, assets)?;
        let mut edits = Vec::new();
        if let Some(model) = &model {
            validate_products(model, assets)?;
            let snapshot = self.document.expand(assets)?;
            let entity = snapshot
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .context("模型对象已失效")?;
            if entity.skin.is_some() {
                let mut candidate = entity.clone();
                candidate.model = Some(model.clone());
                crate::animation::validate_configuration(&candidate, &snapshot, assets)
                    .context("网格与现有蒙皮绑定不兼容")?;
            }
            if matches!(target, FieldTarget::Prefab { .. }) {
                ensure!(entity.model.is_some(), "请在源 Prefab 中维护模型组件结构");
                for (field, value) in [
                    (schema::MESH, FieldValue::Asset(model.mesh)),
                    (schema::MATERIAL, FieldValue::Asset(model.surface.material)),
                    (
                        schema::ROUGHNESS,
                        FieldValue::Scalar(model.surface.roughness),
                    ),
                ] {
                    if schema::descriptor(field)?.read(entity) != Some(value.clone()) {
                        edits.push(FieldEdit {
                            target,
                            field,
                            value,
                        });
                    }
                }
            }
            let transforms = comet_scene::world_transforms(&snapshot)?;
            comet_render::DrawTransform::prepare(transforms[&id])
                .context("对象的世界变换无法用于渲染模型")?;
        } else {
            ensure!(
                matches!(target, FieldTarget::Authored(_)),
                "请在源 Prefab 中维护模型组件结构"
            );
        }
        self.preview_document(assets, |document, assets| match target {
            FieldTarget::Authored(_) => document.preview_model(id, model, assets),
            FieldTarget::Prefab { .. } => document.preview_field_values(&edits, assets),
        })
    }

    pub(crate) fn replace_model_asset(
        &mut self,
        ids: &[EntityId],
        field: FieldKey,
        asset: AssetId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(
            matches!(field, schema::MESH | schema::MATERIAL),
            "只能替换网格或材质资源"
        );
        let mut transforms = None;
        self.preview_common_field(
            ids,
            field,
            FieldValue::Asset(asset),
            assets,
            |candidate, snapshot, assets| {
                if transforms.is_none() {
                    transforms = Some(comet_scene::world_transforms(snapshot)?);
                }
                validate_products(
                    candidate.model.as_ref().context("网格渲染组件已不存在")?,
                    assets,
                )?;
                if candidate.skin.is_some() {
                    crate::animation::validate_configuration(candidate, snapshot, assets)
                        .context("网格与现有蒙皮绑定不兼容")?;
                }
                comet_render::DrawTransform::prepare(transforms.as_ref().unwrap()[&candidate.id])
                    .context("对象的世界变换无法用于渲染模型")?;
                Ok(())
            },
        )
    }
}

#[cfg(test)]
mod tests;
