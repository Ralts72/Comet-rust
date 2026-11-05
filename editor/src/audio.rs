//! Authoring commands validate the selected clip before publishing a document candidate.
use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::EntityId;
use comet_editor_core::{FieldEdit, FieldTarget};
use comet_scene::{
    AudioSource,
    schema::{self, FieldValue},
};

impl Editor {
    pub(crate) fn edit_audio_source(
        &mut self,
        id: EntityId,
        source: Option<AudioSource>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let target = self.resolve_field_target(id, assets)?;
        let mut edits = Vec::new();
        if let Some(source) = &source {
            source.validate()?;
            let product = assets.load(source.clip)?;
            let Product::Audio(data) = &product.payload else {
                anyhow::bail!("请选择 WAV 音频产品");
            };
            data.validate().context("声音引用的音频数据无效")?;
            if matches!(target, FieldTarget::Prefab { .. }) {
                let snapshot = self.document.expand(assets)?;
                let entity = snapshot
                    .entities
                    .iter()
                    .find(|entity| entity.id == id)
                    .context("声音对象已失效")?;
                ensure!(
                    entity.audio_source.is_some(),
                    "请在源 Prefab 中维护声音组件结构"
                );
                for (field, value) in [
                    (schema::AUDIO_CLIP, FieldValue::Asset(source.clip)),
                    (schema::AUDIO_GAIN, FieldValue::Scalar(source.gain)),
                    (schema::AUDIO_LOOPING, FieldValue::Bool(source.looping)),
                    (schema::AUDIO_AUTOPLAY, FieldValue::Bool(source.autoplay)),
                    (schema::AUDIO_SPATIAL, FieldValue::Bool(source.spatial)),
                    (
                        schema::AUDIO_MIN_DISTANCE,
                        FieldValue::Scalar(source.min_distance),
                    ),
                    (
                        schema::AUDIO_MAX_DISTANCE,
                        FieldValue::Scalar(source.max_distance),
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
        } else {
            ensure!(
                matches!(target, FieldTarget::Authored(_)),
                "请在源 Prefab 中维护声音组件结构"
            );
        }
        self.preview_document(assets, |document, assets| match target {
            FieldTarget::Authored(_) => document.preview_audio_source(id, source, assets),
            FieldTarget::Prefab { .. } => document.preview_field_values(&edits, assets),
        })
        .context("无法更新声音组件")
    }
}

#[cfg(test)]
mod tests;
