use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_core::{AssetId, EntityId, InstanceId};
use comet_scene::{EntityData, PrefabResolver};

enum Owner {
    Authored,
    Instance { id: InstanceId, root: EntityId },
}

impl Editor {
    fn structural_edit(&mut self) -> Result<()> {
        ensure!(self.play.is_none(), "请先停止运行，再编辑场景结构");
        ensure!(!self.export.is_running(), "请等待导出完成，再编辑场景结构");
        self.commit()
    }

    fn object_owner(&self, id: EntityId, assets: &mut AssetStore) -> Result<Owner> {
        let source = self.document.shared_source();
        if source.base.entities.iter().any(|e| e.id == id) {
            return Ok(Owner::Authored);
        }
        for instance in &source.instances {
            let prefab = assets.prefab(instance.prefab)?;
            let roots: Vec<_> = prefab
                .base
                .entities
                .iter()
                .filter(|e| e.parent.is_none())
                .collect();
            for root in &roots {
                if root.id.in_instance(instance.id) == id {
                    ensure!(
                        roots.len() == 1
                            && !prefab.instances.iter().any(|child| child.parent.is_none()),
                        "多根 Prefab 请在源文件中编辑结构"
                    );
                    return Ok(Owner::Instance {
                        id: instance.id,
                        root: root.id,
                    });
                }
            }
        }
        anyhow::bail!("Prefab 内部对象的结构需要在源 Prefab 中编辑")
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
                "请在源 Prefab 中创建子对象"
            );
        }
        let source = self.document.shared_source();
        let mut index = 1;
        let name = loop {
            let name = if index == 1 {
                "新对象".to_owned()
            } else {
                format!("新对象 {index}")
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
        self.selected = Some(id);
        self.status = "已创建对象".into();
        Ok(())
    }

    pub(crate) fn place_resource(
        &mut self,
        prefab: AssetId,
        name: &str,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.structural_edit()?;
        let entity = EntityData::new(name);
        let id = entity.id;
        self.document
            .insert_prefab(entity, prefab, assets)
            .context("无法将资源添加到场景")?;
        self.selected = Some(id);
        self.status = format!("已添加资源：{name}");
        Ok(())
    }

    pub(crate) fn duplicate_object(
        &mut self,
        entity: EntityId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.structural_edit()?;
        let selected = match self.object_owner(entity, assets)? {
            Owner::Authored => *self
                .document
                .clone_subtrees(&[entity], assets)?
                .get(&entity)
                .context("对象复制失败")?,
            Owner::Instance { id, root } => {
                root.in_instance(self.document.duplicate_instance(id, assets)?)
            }
        };
        self.selected = Some(selected);
        self.status = "已复制对象".into();
        Ok(())
    }

    pub(crate) fn delete_object(
        &mut self,
        entity: EntityId,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.structural_edit()?;
        match self.object_owner(entity, assets)? {
            Owner::Authored => self.document.remove_subtrees(&[entity], assets)?,
            Owner::Instance { id, .. } => self.document.remove_instance(id, assets)?,
        }
        self.selected = None;
        self.status = "已删除对象".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
