use super::*;
use comet_asset_data::product::AssetKind;
use comet_scene::schema::FieldKey;

const FIELDS: &[FieldKey] = &[
    schema::ANIMATOR_SKELETON,
    schema::ANIMATOR_CLIP,
    schema::ANIMATOR_CONTROLLER,
    schema::ANIMATOR_BINDINGS,
];
enum RigEdit {
    Skeleton(AssetId),
    RebuildBindings,
}
impl Editor {
    pub(crate) fn replace_animation_skeletons(
        &mut self,
        ids: &[EntityId],
        skeleton: AssetId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.preview_rigs(ids, RigEdit::Skeleton(skeleton), assets)
    }
    pub(crate) fn rebuild_animation_bindings(
        &mut self,
        ids: &[EntityId],
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.preview_rigs(ids, RigEdit::RebuildBindings, assets)
    }
    fn preview_rigs(
        &mut self,
        ids: &[EntityId],
        edit: RigEdit,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        if ids.is_empty() {
            return Ok(());
        }
        let before = self.document.expand(assets)?;
        let mut candidate = before.clone();
        let mut targets = BTreeSet::new();
        let mut edits = Vec::new();
        for &id in ids {
            ensure!(targets.insert(id), "骨架编辑目标重复：{id}");
            let target = self.resolve_field_target(id, assets)?;
            let index = before
                .entities
                .binary_search_by_key(&id, |e| e.id)
                .map_err(|_| anyhow::anyhow!("骨架对象已失效：{id}"))?;
            let original = &before.entities[index];
            let next = &mut candidate.entities[index];
            let animator = next
                .animator
                .as_mut()
                .with_context(|| format!("对象 {} ({id}) 缺少动画组件", next.name))?;
            let skeleton = match edit {
                RigEdit::Skeleton(skeleton) => skeleton,
                RigEdit::RebuildBindings => animator.skeleton,
            };
            let product = assets.load(skeleton)?;
            let Product::Skeleton(data) = &product.payload else {
                anyhow::bail!("请选择骨架产品");
            };
            let changed = skeleton != animator.skeleton;
            if changed || matches!(edit, RigEdit::RebuildBindings) {
                animator.bindings = data
                    .nodes
                    .iter()
                    .map(|node| (node.id, animator.bindings.get(&node.id).cloned().flatten()))
                    .collect();
            }
            if changed {
                animator.skeleton = skeleton;
                // Only dependencies explicitly targeting the chosen skeleton remain playable.
                let compatible = |id: AssetId, kind| {
                    assets.index().products.get(&id).is_some_and(|entry| {
                        entry.kind == kind && entry.dependencies.contains(&skeleton)
                    })
                };
                animator.clip = animator
                    .clip
                    .filter(|id| compatible(*id, AssetKind::AnimationClip));
                animator.controller = animator
                    .controller
                    .filter(|id| compatible(*id, AssetKind::AnimationController));
            }
            for &field in FIELDS {
                let descriptor = schema::descriptor(field)?;
                if descriptor.read(original) != descriptor.read(next) {
                    let mut value = descriptor.read(next).context("动画字段已不存在")?;
                    self.source_field_value(target, &mut value, assets)?;
                    edits.push(FieldEdit {
                        target,
                        field,
                        value,
                    });
                }
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        validate_rig_changes(&candidate, &targets, assets)?;
        self.preview_document(assets, |document, assets| {
            document.preview_field_values(&edits, assets)
        })
    }
}
