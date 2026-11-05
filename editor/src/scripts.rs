use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::EntityId;
use comet_scene::{Script, schema::FieldValue};

impl Editor {
    pub(crate) fn edit_script(
        &mut self,
        id: EntityId,
        mut script: Option<Script>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        if let Some(script) = &mut script {
            ensure!(
                matches!(&assets.load(script.asset)?.payload, Product::Script(_)),
                "脚本组件必须引用脚本资源"
            );
            let target = self.resolve_field_target(id, assets)?;
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
