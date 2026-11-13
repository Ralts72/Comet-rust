use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::{AssetId, EntityId};
use comet_editor_core::FieldTarget;
use comet_scene::{
    FieldUpdate, Script,
    schema::{self, FieldValue},
};

pub(crate) fn validate_asset(asset: AssetId, assets: &mut AssetStore) -> Result<()> {
    ensure!(
        matches!(&assets.load(asset)?.payload, Product::Script(_)),
        "脚本组件必须引用脚本资源"
    );
    Ok(())
}

impl Editor {
    pub(crate) fn replace_script_assets(
        &mut self,
        ids: &[EntityId],
        asset: AssetId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.preview_common_fields(
            ids,
            &[(schema::SCRIPT_ASSET, FieldValue::Asset(asset))],
            assets,
            |candidate, _, assets| {
                let script = candidate.script.as_ref().context("脚本组件已不存在")?;
                script.validate()?;
                validate_asset(script.asset, assets)
            },
        )
    }

    pub(crate) fn edit_script(
        &mut self,
        id: EntityId,
        mut script: Option<Script>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let target = self.resolve_field_target(id, assets)?;
        if let Some(script) = &script {
            validate_asset(script.asset, assets)?;
        }
        if matches!(target, FieldTarget::Prefab { .. }) {
            let next = script
                .as_ref()
                .context("Prefab 的脚本组件结构请在源 Prefab 中编辑")?;
            let snapshot = self.document.expand(assets)?;
            let before = snapshot
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .and_then(|entity| entity.script.as_ref())
                .context("Prefab 的脚本组件结构请在源 Prefab 中编辑")?;
            ensure!(
                before.parameters == next.parameters,
                "Prefab 的脚本参数请在源 Prefab 中编辑"
            );
            let updates = [
                (
                    schema::SCRIPT_ASSET,
                    FieldValue::Asset(before.asset),
                    FieldValue::Asset(next.asset),
                ),
                (
                    schema::SCRIPT_ENABLED,
                    FieldValue::Bool(before.enabled),
                    FieldValue::Bool(next.enabled),
                ),
            ]
            .into_iter()
            .filter_map(|(field, old, value)| {
                (old != value).then_some(FieldUpdate {
                    entity: id,
                    field,
                    value,
                })
            })
            .collect::<Vec<_>>();
            if updates.is_empty() {
                return Ok(());
            }
            return self.edit_fields(&updates, assets);
        }
        if let Some(script) = &mut script {
            let mut value = FieldValue::ScriptParameters(std::mem::take(&mut script.parameters));
            self.source_field_value(target, &mut value, assets)?;
            let FieldValue::ScriptParameters(parameters) = value else {
                unreachable!()
            };
            script.parameters = parameters;
        }
        self.preview_document(assets, |document, assets| {
            document.preview_script(id, script, assets)
        })
        .context("无法更新脚本组件，请检查参数及实体引用")
    }
}

#[cfg(test)]
mod tests;
