use super::{CloneMap, Document, ObjectTarget, selection::capture_objects};
use anyhow::{Result, ensure};
use comet_core::{EntityId, InstanceId, WorldId};
use comet_scene::{ContentSource, EntityRef, PrefabResolver, schema};
use std::collections::BTreeSet;

/// An immutable authoring fragment. References outside it still belong to the original world.
/// Asset products and expanded prefab entities remain owned by their respective sources.
pub struct ObjectSnapshot {
    content: ContentSource,
    origin: WorldId,
    requires_origin: bool,
}

impl ObjectSnapshot {
    /// Authoring declarations for host-side resource validation; never a mutable clipboard view.
    pub fn source(&self) -> &ContentSource {
        &self.content
    }
}

impl Document {
    /// Capture authored subtrees and directly declared instances without changing history.
    pub fn capture_selection(
        &self,
        selection: &[ObjectTarget],
        resolver: &mut impl PrefabResolver,
    ) -> Result<ObjectSnapshot> {
        ensure!(self.preview.is_none(), "请先结束当前字段编辑，再复制实体");
        ensure!(!selection.is_empty(), "请选择要复制的实体");
        self.expand(resolver)?;
        let content = capture_objects(&self.current.source, selection)?;
        let entities: BTreeSet<_> = content.base.entities.iter().map(|e| e.id).collect();
        let instances: BTreeSet<_> = content.instances.iter().map(|i| i.id).collect();
        // Override references use each prefab's own namespace, so only author fields are checked.
        let requires_origin = content.base.entities.iter().any(|entity| {
            schema::entity_references(entity)
                .iter()
                .any(|reference| match reference {
                    EntityRef::Local { entity } => !entities.contains(entity),
                    EntityRef::InInstance { instance, .. } => !instances.contains(instance),
                    EntityRef::External { .. } => false,
                })
        });
        Ok(ObjectSnapshot {
            content,
            origin: self.world_id(),
            requires_origin,
        })
    }

    /// Paste a fresh identity namespace at an authored parent (or scene root), keeping local poses.
    /// Complete expansion validates root constraints, source cycles and references before publication.
    pub fn paste_selection(
        &mut self,
        snapshot: &ObjectSnapshot,
        parent: Option<EntityId>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<CloneMap> {
        ensure!(
            !snapshot.requires_origin || snapshot.origin == self.world_id(),
            "复制内容引用了所选组以外的本地实体；跨文档粘贴时请一起复制被引用对象"
        );
        self.change(resolver, |next| {
            ensure!(
                parent.is_none_or(|id| next.base.entities.iter().any(|e| e.id == id)),
                "粘贴父级必须是当前文档的作者实体"
            );
            let mut content = snapshot.content.clone();
            let map = CloneMap {
                entities: content
                    .base
                    .entities
                    .iter()
                    .map(|e| (e.id, EntityId::new()))
                    .collect(),
                instances: content
                    .instances
                    .iter()
                    .map(|i| (i.id, InstanceId::new()))
                    .collect(),
            };
            for entity in &mut content.base.entities {
                entity.id = map.entities[&entity.id];
                entity.parent = entity.parent.map(|id| map.entities[&id]).or(parent);
                schema::remap(entity, &map.entities)?;
                schema::remap_instance_references(entity, &map.instances)?;
            }
            for instance in &mut content.instances {
                instance.id = map.instances[&instance.id];
                instance.parent = instance.parent.map(|id| map.entities[&id]).or(parent);
            }
            next.base.entities.extend(content.base.entities);
            next.instances.extend(content.instances);
            Ok(map)
        })
    }
}
