//! Shared field targeting and authoring commands; transforms and component panels use the same route.
use crate::resources::ResourceVersion;
use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_core::EntityId;
use comet_editor_core::{Document, FieldTarget};
use comet_scene::{
    Character, EntityRef, FieldUpdate, PhysicsBody,
    schema::{self, FieldKey, FieldValue},
};
use std::{collections::BTreeMap, sync::Arc};
mod bindings;
pub(crate) use bindings::accepts_binding;

pub(crate) struct FieldReferences {
    pub target: FieldTarget,
    pub targets: Vec<(EntityId, String)>,
    references: BTreeMap<EntityId, EntityRef>,
}

impl FieldReferences {
    fn write_reference(&self, reference: &mut EntityRef) -> Result<()> {
        match reference {
            EntityRef::Local { entity } => {
                *reference = self
                    .references
                    .get(entity)
                    .context("目标不属于当前作者对象或允许的直接 Prefab 作用域")?
                    .clone();
            }
            EntityRef::InInstance { .. } => ensure!(
                self.references
                    .values()
                    .any(|allowed| allowed == &*reference),
                if matches!(self.target, FieldTarget::Authored(_)) {
                    "此字段不能引用其他实例或嵌套 Prefab 作用域"
                } else {
                    "实例覆盖只能保留已有的子实例引用"
                }
            ),
            EntityRef::External { .. } => {}
        }
        Ok(())
    }

    pub(crate) fn write_value(&self, value: &mut FieldValue) -> Result<()> {
        schema::map_value_entity_references(value, |reference| self.write_reference(reference))
    }
}

struct ReferenceCache {
    version: ResourceVersion,
    owner: EntityId,
    value: std::result::Result<Arc<FieldReferences>, String>,
}

impl Editor {
    pub(crate) fn ensure_authoring_editable(&self) -> Result<()> {
        ensure!(self.play.is_none(), "请先停止运行，再修改作者文档");
        ensure!(!self.is_busy(), "请等待后台任务完成，再修改作者文档");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        Ok(())
    }

    /// A failed first preview closes only the transaction it opened; an ongoing gesture survives.
    pub(crate) fn preview_document(
        &mut self,
        assets: &mut AssetStore,
        edit: impl FnOnce(&mut Document, &mut AssetStore) -> Result<()>,
    ) -> Result<()> {
        let began = !self.document.is_editing();
        if began {
            self.picking.clear();
            self.document.begin()?;
        }
        let result = edit(&mut self.document, assets);
        if result.is_err() && began {
            self.document.cancel()?;
        }
        result
    }

    pub fn edit_physics(
        &mut self,
        id: EntityId,
        body: Option<PhysicsBody>,
        character: Option<Character>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        self.preview_document(assets, |document, assets| {
            document.preview_physics(id, body, character, assets)
        })
        .context("无法更新物理组件，请检查对象层级、缩放及角色配置")
    }

    /// UI selections use expanded IDs; persistence always uses the owner's lexical source scope.
    pub(crate) fn field_references(
        &self,
        target: FieldTarget,
        assets: &mut AssetStore,
    ) -> Result<FieldReferences> {
        let source = self.document.shared_source();
        let mut references = BTreeMap::new();
        let mut targets = Vec::new();
        match target {
            FieldTarget::Authored(_) => {
                for entity in &source.base.entities {
                    references.insert(entity.id, EntityRef::local(entity.id));
                    targets.push((entity.id, entity.name.clone()));
                }
                for (ordinal, instance) in source.instances.iter().enumerate() {
                    let product = assets.load(instance.prefab)?;
                    let Product::Prefab(prefab) = &product.payload else {
                        anyhow::bail!("实例资源不是 Prefab");
                    };
                    let label = instance
                        .parent
                        .and_then(|parent| {
                            source
                                .base
                                .entities
                                .iter()
                                .find(|entity| entity.id == parent)
                        })
                        .map(|entity| entity.name.as_str())
                        .or_else(|| {
                            assets
                                .index()
                                .products
                                .get(&instance.prefab)
                                .and_then(|entry| entry.editor.as_ref())
                                .map(|editor| editor.label.as_str())
                        })
                        .unwrap_or("Prefab");
                    for entity in &prefab.base.entities {
                        let world = entity.id.in_instance(instance.id);
                        references.insert(
                            world,
                            EntityRef::InInstance {
                                instance: instance.id,
                                entity: entity.id,
                            },
                        );
                        targets.push((
                            world,
                            format!("{label} [{}] / {}", ordinal + 1, entity.name),
                        ));
                    }
                }
            }
            FieldTarget::Prefab {
                instance,
                entity: owner,
            } => {
                let declaration = source
                    .instances
                    .iter()
                    .find(|value| value.id == instance)
                    .ok_or_else(|| anyhow::anyhow!("Prefab 实例已不存在"))?;
                let product = assets.load(declaration.prefab)?;
                let Product::Prefab(prefab) = &product.payload else {
                    anyhow::bail!("实例资源不是 Prefab");
                };
                for entity in &prefab.base.entities {
                    let world = entity.id.in_instance(instance);
                    references.insert(world, EntityRef::local(entity.id));
                    targets.push((world, entity.name.clone()));
                }
                let mut owner = prefab
                    .base
                    .entities
                    .iter()
                    .find(|entity| entity.id == owner)
                    .context("Prefab 对象已不存在")?
                    .clone();
                let owner_id = owner.id;
                for value in declaration
                    .overrides
                    .iter()
                    .filter(|value| value.entity == owner_id)
                {
                    schema::descriptor(value.field)?.write(&mut owner, value.value.clone())?;
                }
                // Aggregate fields must preserve inherited child references without exposing
                // those nested objects as new picker choices.
                for reference in schema::entity_references(&owner) {
                    if let EntityRef::InInstance {
                        instance: child,
                        entity,
                    } = reference
                    {
                        references
                            .insert(entity.in_instance(child.in_instance(instance)), reference);
                    }
                }
            }
        }
        Ok(FieldReferences {
            target,
            targets,
            references,
        })
    }

    pub(crate) fn cached_field_references(
        &self,
        ctx: &egui::Context,
        owner: EntityId,
        assets: &mut AssetStore,
    ) -> Result<Arc<FieldReferences>> {
        ensure!(self.play.is_none(), "运行期不解析作者引用作用域");
        let key = egui::Id::unique("inspector-field-reference-targets");
        let cached = ctx
            .data(|data| data.get_temp::<Arc<ReferenceCache>>(key))
            .filter(|cache| cache.version.matches(&self.document, assets) && cache.owner == owner);
        let cache = cached.unwrap_or_else(|| {
            let value = self
                .resolve_field_target(owner, assets)
                .and_then(|target| self.field_references(target, assets))
                .map(Arc::new)
                .map_err(|error| format!("{error:#}"));
            let cache = Arc::new(ReferenceCache {
                version: ResourceVersion::capture(&self.document, assets),
                owner,
                value,
            });
            ctx.data_mut(|data| data.insert_temp(key, cache.clone()));
            cache
        });
        cache.value.clone().map_err(anyhow::Error::msg)
    }

    pub(crate) fn source_field_value(
        &self,
        target: FieldTarget,
        value: &mut FieldValue,
        assets: &mut AssetStore,
    ) -> Result<()> {
        if !schema::value_entity_references(value).is_empty() {
            self.field_references(target, assets)?.write_value(value)?;
        }
        Ok(())
    }

    /// Expanded roots include instance placement; overrides remain in the prefab's source space.
    pub(crate) fn source_translation_value(
        &self,
        target: FieldTarget,
        field: FieldKey,
        value: &mut FieldValue,
        assets: &mut AssetStore,
    ) -> Result<()> {
        if field != schema::TRANSLATION {
            return Ok(());
        }
        let FieldTarget::Prefab { instance, entity } = target else {
            return Ok(());
        };
        let source = self.document.shared_source();
        let declaration = source
            .instances
            .iter()
            .find(|i| i.id == instance)
            .context("Prefab 实例已不存在")?;
        if declaration.translation == [0.; 3] {
            return Ok(());
        }
        let product = assets.load(declaration.prefab)?;
        let Product::Prefab(prefab) = &product.payload else {
            anyhow::bail!("实例资源不是 Prefab")
        };
        if let Some(root) = prefab
            .base
            .entities
            .iter()
            .find(|e| e.id == entity && e.parent.is_none())
        {
            let FieldValue::Vec3(position) = value else {
                anyhow::bail!("无效的位移字段")
            };
            let initial = match declaration
                .overrides
                .iter()
                .find(|o| o.entity == entity && o.field == field)
                .map(|o| &o.value)
            {
                Some(FieldValue::Vec3(value)) => *value,
                None => root.transform.translation,
                Some(_) => anyhow::bail!("无效的位移覆盖"),
            };
            let expanded =
                std::array::from_fn(|axis| initial[axis] + declaration.translation[axis]);
            *position = if *position == expanded {
                initial
            } else {
                std::array::from_fn(|axis| position[axis] - declaration.translation[axis])
            };
            ensure!(
                position.iter().all(|v| v.is_finite()),
                "资源位置超出有效范围"
            );
        }
        Ok(())
    }

    pub(crate) fn resolve_field_target(
        &self,
        id: EntityId,
        assets: &mut AssetStore,
    ) -> Result<FieldTarget> {
        let source = self.document.shared_source();
        if source.base.entities.iter().any(|e| e.id == id) {
            return Ok(FieldTarget::Authored(id));
        }
        for instance in &source.instances {
            let product = assets.load(instance.prefab)?;
            let Product::Prefab(prefab) = &product.payload else {
                anyhow::bail!("实例资源不是 Prefab");
            };
            for entity in &prefab.base.entities {
                if entity.id.in_instance(instance.id) == id {
                    return Ok(FieldTarget::Prefab {
                        instance: instance.id,
                        entity: entity.id,
                    });
                }
            }
        }
        anyhow::bail!("此对象暂不支持嵌套结构编辑")
    }

    pub fn edit(
        &mut self,
        id: EntityId,
        key: FieldKey,
        value: FieldValue,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.edit_fields(
            &[FieldUpdate {
                entity: id,
                field: key,
                value,
            }],
            assets,
        )
    }

    pub(crate) fn edit_fields(
        &mut self,
        updates: &[FieldUpdate],
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再编辑场景");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        if updates.is_empty() {
            return Ok(());
        }
        if let Some(play) = &mut self.play {
            return play.set_fields(updates);
        }
        let edits = updates
            .iter()
            .map(|update| {
                let target = self
                    .resolve_field_target(update.entity, assets)
                    .with_context(|| format!("无法编辑实体 {} 的字段", update.entity))?;
                let mut value = update.value.clone();
                self.source_field_value(target, &mut value, assets)?;
                self.source_translation_value(target, update.field, &mut value, assets)?;
                Ok(comet_editor_core::FieldEdit {
                    target,
                    field: update.field,
                    value,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        self.preview_document(assets, |document, assets| {
            document.preview_field_values(&edits, assets)
        })
    }

    /// Existing-field assignment validates every target before opening a single authoring preview.
    pub(crate) fn preview_common_fields(
        &mut self,
        ids: &[EntityId],
        values: &[(FieldKey, FieldValue)],
        assets: &mut AssetStore,
        mut validate: impl FnMut(
            &comet_scene::EntityData,
            &comet_scene::SceneSnapshot,
            &mut AssetStore,
        ) -> Result<()>,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let mut fields = std::collections::BTreeSet::new();
        for (field, value) in values {
            ensure!(
                fields.insert((field.component, field.field)),
                "共同属性字段重复"
            );
            schema::descriptor(*field)?.validate_value(value)?;
        }
        if ids.is_empty() || values.is_empty() {
            return Ok(());
        }
        let snapshot = self.document.expand(assets)?;
        let mut unique = std::collections::BTreeSet::new();
        let mut edits = Vec::new();
        for &id in ids {
            ensure!(unique.insert(id), "共同属性目标重复：{id}");
            // Unchanged nested members must still reject the whole selection.
            let target = self
                .resolve_field_target(id, assets)
                .with_context(|| format!("无法编辑实体 {id} 的共同属性"))?;
            let index = snapshot
                .entities
                .binary_search_by_key(&id, |entity| entity.id)
                .map_err(|_| anyhow::anyhow!("共同属性对象已失效：{id}"))?;
            let before = &snapshot.entities[index];
            let mut candidate = before.clone();
            for (field, value) in values {
                let descriptor = schema::descriptor(*field)?;
                let initial = descriptor.read(before).with_context(|| {
                    format!(
                        "实体 {} ({id}) 缺少共同属性 {}",
                        before.name, descriptor.path
                    )
                })?;
                descriptor.write(&mut candidate, value.clone())?;
                if initial != *value {
                    let mut source_value = value.clone();
                    self.source_field_value(target, &mut source_value, assets)?;
                    self.source_translation_value(target, *field, &mut source_value, assets)?;
                    edits.push(comet_editor_core::FieldEdit {
                        target,
                        field: *field,
                        value: source_value,
                    });
                }
            }
            validate(&candidate, &snapshot, assets)
                .with_context(|| format!("实体 {} ({id}) 的共同属性候选无效", before.name))?;
        }
        if edits.is_empty() {
            return Ok(());
        }
        self.preview_document(assets, |document, assets| {
            document.preview_field_values(&edits, assets)
        })
    }

    pub(crate) fn revert_prefab_fields(
        &mut self,
        ids: &[EntityId],
        fields: Option<&[FieldKey]>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let mut keys = std::collections::BTreeSet::new();
        for &field in fields.unwrap_or(&[]) {
            schema::descriptor(field)?;
            ensure!(
                keys.insert((field.component, field.field)),
                "实例覆盖字段重复"
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut resets = Vec::new();
        for &id in ids {
            ensure!(seen.insert(id), "实例覆盖目标重复：{id}");
            let FieldTarget::Prefab { instance, entity } = self
                .resolve_field_target(id, assets)
                .with_context(|| format!("无法恢复实体 {id} 的继承"))?
            else {
                anyhow::bail!("恢复继承仅适用于直接 Prefab 实体");
            };
            let fields = self
                .document
                .shared_source()
                .instances
                .iter()
                .find(|value| value.id == instance)
                .unwrap()
                .overrides
                .iter()
                .filter(|value| {
                    value.entity == entity
                        && fields.is_none_or(|fields| fields.contains(&value.field))
                })
                .map(|value| value.field)
                .collect();
            resets.push(comet_editor_core::OverrideReset {
                instance,
                entity,
                fields,
            });
        }
        self.commit()?;
        self.document.revert_overrides_many(&resets, assets)?;
        self.picking.clear();
        self.status = "已恢复所选 Prefab 字段继承".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
