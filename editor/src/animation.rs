//! Authoring commands keep rig references in the document's declared identity space.
use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::{AssetId, EntityId};
use comet_editor_core::{FieldEdit, FieldTarget};
use comet_scene::{Animator, EntityData, EntityRef, SceneSnapshot, SkinBinding, schema};
use std::collections::BTreeSet;

impl Editor {
    pub(crate) fn replace_animation_clips(
        &mut self,
        ids: &[EntityId],
        clip: Option<AssetId>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.preview_common_field(
            ids,
            schema::ANIMATOR_CLIP,
            schema::FieldValue::OptionalAsset(clip),
            assets,
            validate_configuration,
        )
    }

    pub(crate) fn edit_animation(
        &mut self,
        id: EntityId,
        animator: Option<Animator>,
        skin: Option<SkinBinding>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let target = self.resolve_field_target(id, assets)?;
        let snapshot = self.document.expand(assets)?;
        let before = snapshot
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .context("动画对象已不存在")?;
        let mut after = before.clone();
        after.animator = animator;
        after.skin = skin;
        if before.animator == after.animator && before.skin == after.skin {
            return Ok(());
        }
        validate_configuration(&after, &snapshot, assets)?;
        let mut edits = Vec::new();
        if matches!(target, FieldTarget::Prefab { .. }) {
            ensure!(
                before.animator.is_some() == after.animator.is_some()
                    && before.skin.is_some() == after.skin.is_some(),
                "请在源 Prefab 中维护动画组件结构"
            );
            for field in [
                schema::ANIMATOR_SKELETON,
                schema::ANIMATOR_CLIP,
                schema::ANIMATOR_BINDINGS,
                schema::ANIMATOR_LOOPING,
                schema::ANIMATOR_SPEED,
                schema::SKIN_ASSET,
                schema::SKIN_ANIMATOR,
            ] {
                let descriptor = schema::descriptor(field)?;
                let value = descriptor.read(&after);
                if descriptor.read(before) != value
                    && let Some(mut value) = value
                {
                    self.source_field_value(target, &mut value, assets)?;
                    edits.push(FieldEdit {
                        target,
                        field,
                        value,
                    });
                }
            }
        } else {
            for field in [schema::ANIMATOR_BINDINGS, schema::SKIN_ANIMATOR] {
                let descriptor = schema::descriptor(field)?;
                if let Some(mut value) = descriptor.read(&after) {
                    self.source_field_value(target, &mut value, assets)?;
                    descriptor.write(&mut after, value)?;
                }
            }
        }
        self.preview_document(assets, |document, assets| match target {
            FieldTarget::Authored(_) => {
                document.preview_animation(id, after.animator, after.skin, assets)
            }
            FieldTarget::Prefab { .. } => document.preview_field_values(&edits, assets),
        })
        .context("无法更新动画配置")
    }
}

pub(crate) fn validate_configuration(
    after: &EntityData,
    snapshot: &SceneSnapshot,
    assets: &mut AssetStore,
) -> Result<()> {
    if let Some(animator) = &after.animator {
        animator.validate()?;
        let product = assets.load(animator.skeleton)?;
        let Product::Skeleton(skeleton) = &product.payload else {
            anyhow::bail!("请选择骨架产品");
        };
        let nodes: BTreeSet<_> = skeleton.nodes.iter().map(|node| node.id).collect();
        ensure!(
            animator.bindings.keys().all(|node| nodes.contains(node)),
            "绑定包含不属于当前骨架的源节点"
        );
        if let Some(clip) = animator.clip {
            let product = assets.load(clip)?;
            let Product::AnimationClip(clip) = &product.payload else {
                anyhow::bail!("请选择动画片段产品");
            };
            ensure!(
                clip.skeleton == animator.skeleton,
                "动画片段与当前骨架不匹配"
            );
        }
    }
    if let Some(binding) = &after.skin {
        let model = after.model.as_ref().context("蒙皮绑定需要模型组件")?;
        let product = assets.load(binding.skin)?;
        let Product::Skin(skin) = &product.payload else {
            anyhow::bail!("请选择蒙皮产品");
        };
        let mesh = assets.load(model.mesh)?;
        let Product::Mesh(mesh) = &mesh.payload else {
            anyhow::bail!("模型引用的产品不是网格");
        };
        skin.validate_mesh(mesh).context("网格与所选蒙皮不匹配")?;
        if let Some(reference) = &binding.animator {
            let EntityRef::Local { entity } = reference else {
                anyhow::bail!("蒙皮目前只支持当前场景中的动画控制器");
            };
            let owner = if *entity == after.id {
                after
            } else {
                snapshot
                    .entities
                    .iter()
                    .find(|candidate| candidate.id == *entity)
                    .context("动画控制器对象已不存在")?
            };
            if let Some(animator) = &owner.animator {
                ensure!(
                    animator.skeleton == skin.skeleton,
                    "蒙皮与动画控制器的骨架不匹配"
                );
            }
        }
    }
    Ok(())
}
