use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_editor_core::ObjectTarget;
use comet_scene::{EntityData, PrefabResolver, PrefabRoot};
use std::collections::{BTreeMap, btree_map::Entry};

enum Owner {
    Authored,
    Instance {
        id: InstanceId,
        roots: Vec<PrefabRoot>,
    },
}

impl Editor {
    pub(crate) fn rename_object(&mut self, id: EntityId, name: String) -> Result<()> {
        ensure!(self.play.is_none(), "请先停止运行，再重命名作者实体");
        ensure!(!self.is_busy(), "请等待后台任务完成，再重命名实体");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        ensure!(
            self.document
                .shared_source()
                .base
                .entities
                .iter()
                .any(|entity| entity.id == id),
            "Prefab 内部实体的名称需要在源文件中编辑"
        );
        if !self.document.is_editing() {
            self.picking.clear();
            self.document.begin()?;
        }
        self.document.preview_name(id, name)
    }

    fn ensure_structural_editable(&self) -> Result<()> {
        ensure!(self.play.is_none(), "请先停止运行，再编辑场景结构");
        ensure!(!self.is_busy(), "请等待后台任务完成，再编辑场景结构");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        Ok(())
    }

    fn structural_edit(&mut self) -> Result<()> {
        self.ensure_structural_editable()?;
        self.picking.clear();
        self.commit()
    }

    fn object_owner(&self, id: EntityId, assets: &mut AssetStore) -> Result<Owner> {
        let source = self.document.shared_source();
        if source.base.entities.iter().any(|e| e.id == id) {
            return Ok(Owner::Authored);
        }
        self.instance_owners(assets)?
            .remove(&id)
            .context("Prefab 内部实体的结构需要在源 Prefab 中编辑")
    }

    fn instance_owners(&self, assets: &mut AssetStore) -> Result<BTreeMap<EntityId, Owner>> {
        // This command-local index shares source metadata between instances and selected roots.
        let mut sources = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for instance in &self.document.shared_source().instances {
            if let Entry::Vacant(entry) = sources.entry(instance.prefab) {
                let prefab = assets.prefab(instance.prefab)?;
                entry.insert(prefab.prefab_roots(assets)?);
            }
            for root in &sources[&instance.prefab] {
                owners.insert(
                    root.in_instance(instance.id),
                    Owner::Instance {
                        id: instance.id,
                        roots: vec![root.clone()],
                    },
                );
            }
        }
        Ok(owners)
    }

    pub(crate) fn create_object(
        &mut self,
        parent: Option<EntityId>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.structural_edit()?;
        if let Some(id) = parent {
            ensure!(
                matches!(self.object_owner(id, assets)?, Owner::Authored),
                "请在源 Prefab 中创建子实体"
            );
        }
        let source = self.document.shared_source();
        let mut index = 1;
        let name = loop {
            let name = if index == 1 {
                "新实体".to_owned()
            } else {
                format!("新实体 {index}")
            };
            if !source.base.entities.iter().any(|e| e.name == name) {
                break name;
            }
            index += 1;
        };
        let mut entity = EntityData::new(name);
        entity.parent = parent;
        let id = entity.id;
        self.document.insert_entity(entity, assets)?;
        self.selection.replace([id]);
        self.status = "已创建实体".into();
        Ok(())
    }

    pub(crate) fn place_resource(
        &mut self,
        prefab: AssetId,
        name: &str,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_structural_editable()?;
        let source = assets.prefab(prefab)?;
        let roots = source.prefab_roots(assets)?;
        let unparented = roots.iter().any(PrefabRoot::requires_scene_root);
        let entity = (!unparented).then(|| EntityData::new(name));
        let id = entity.as_ref().map(|entity| entity.id);
        self.structural_edit()?;
        let instance = self
            .document
            .insert_prefab(entity, prefab, assets)
            .context("无法将资源添加到场景")?;
        if unparented {
            self.selection
                .replace(roots.iter().map(|root| root.in_instance(instance)));
        } else {
            self.selection.replace(id);
        }
        self.status = format!("已添加资源：{name}");
        Ok(())
    }

    pub(crate) fn duplicate_object(
        &mut self,
        entity: EntityId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.structural_edit()?;
        let owners = self.operation_roots(entity, assets)?;
        let targets: Vec<_> = owners.iter().map(|(id, owner)| owner.target(*id)).collect();
        let map = self.document.clone_selection(&targets, assets)?;
        let mut selected = Vec::new();
        for (entity, owner) in owners {
            match owner {
                Owner::Authored => selected.push(map.entities[&entity]),
                Owner::Instance { id, roots } => selected.extend(
                    roots
                        .iter()
                        .map(|root| root.in_instance(map.instances[&id])),
                ),
            }
        }
        self.selection.replace(selected);
        self.status = "已复制实体".into();
        Ok(())
    }

    pub(crate) fn delete_object(
        &mut self,
        entity: EntityId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.structural_edit()?;
        let owners = self.operation_roots(entity, assets)?;
        let targets: Vec<_> = owners.iter().map(|(id, owner)| owner.target(*id)).collect();
        self.document.remove_selection(&targets, assets)?;
        self.selection.clear();
        self.status = "已删除实体".into();
        Ok(())
    }

    pub(crate) fn reparent_object(
        &mut self,
        entity: EntityId,
        parent: Option<EntityId>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.structural_edit()?;
        let owners = self.operation_roots(entity, assets)?;
        ensure!(
            owners
                .iter()
                .all(|(_, owner)| matches!(owner, Owner::Authored)),
            "请移动资源的作者容器，Prefab 的内部结构需要在源文件中编辑"
        );
        if let Some(parent) = parent {
            ensure!(
                matches!(self.object_owner(parent, assets)?, Owner::Authored),
                "不能将实体挂接到 Prefab 内部，请选择作者实体"
            );
        }
        self.document
            .reparent_entities(
                &owners.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                parent,
                assets,
            )
            .context("无法调整父级；请检查层级循环、物理根节点和变换是否可保留")?;
        if !self.selection.contains(entity) {
            self.selection.replace([entity]);
        }
        self.status = "父级已更新，世界变换保持不变".into();
        Ok(())
    }

    fn operation_roots(
        &self,
        clicked: EntityId,
        assets: &mut AssetStore,
    ) -> Result<Vec<(EntityId, Owner)>> {
        let mut ids = if self.selection.contains(clicked) {
            self.selection.ids()
        } else {
            vec![clicked]
        };
        // Keep the clicked/active root last so copies retain a predictable Gizmo anchor.
        ids.retain(|id| *id != clicked);
        ids.push(clicked);
        let selected: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        let snapshot = self.document.expand(assets)?;
        let parents: std::collections::BTreeMap<_, _> =
            snapshot.entities.iter().map(|e| (e.id, e.parent)).collect();
        let mut roots: Vec<(EntityId, Owner)> = Vec::new();
        let authored: std::collections::BTreeSet<_> = self
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .map(|entity| entity.id)
            .collect();
        let mut instances = None;
        for id in ids {
            let mut parent = *parents.get(&id).context("选中的实体已不存在")?;
            let mut covered = false;
            while let Some(ancestor) = parent {
                if selected.contains(&ancestor) {
                    covered = true;
                    break;
                }
                parent = parents[&ancestor];
            }
            if !covered {
                let mut owner = if authored.contains(&id) {
                    Owner::Authored
                } else {
                    if instances.is_none() {
                        instances = Some(self.instance_owners(assets)?);
                    }
                    instances
                        .as_mut()
                        .unwrap()
                        .remove(&id)
                        .context("Prefab 内部实体的结构需要在源 Prefab 中编辑")?
                };
                if let Owner::Instance {
                    id: instance,
                    roots: selected,
                } = &mut owner
                    && let Some(index) = roots.iter().position(|(_, previous)| matches!(previous, Owner::Instance { id, .. } if *id == *instance))
                {
                    let (_, Owner::Instance { roots: mut previous, .. }) = roots.remove(index) else { unreachable!() };
                    // The clicked root is last, including within a merged instance selection.
                    previous.append(selected);
                    *selected = previous;
                }
                roots.push((id, owner));
            }
        }
        Ok(roots)
    }
}

impl Owner {
    fn target(&self, entity: EntityId) -> ObjectTarget {
        match self {
            Self::Authored => ObjectTarget::Authored(entity),
            Self::Instance { id, .. } => ObjectTarget::Prefab(*id),
        }
    }
}

#[cfg(test)]
mod tests;
