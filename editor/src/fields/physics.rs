use super::*;
use comet_editor_core::FieldEdit;
use comet_scene::ColliderShape;
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColliderKind {
    Box,
    Sphere,
    Capsule,
}
impl ColliderKind {
    pub(crate) fn of(shape: &ColliderShape) -> Self {
        match shape {
            ColliderShape::Box { .. } => Self::Box,
            ColliderShape::Sphere { .. } => Self::Sphere,
            ColliderShape::Capsule { .. } => Self::Capsule,
        }
    }
    fn initial(self) -> ColliderShape {
        match self {
            Self::Box => ColliderShape::Box {
                half_extents: [0.5; 3],
            },
            Self::Sphere => ColliderShape::Sphere { radius: 0.5 },
            Self::Capsule => ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.3,
            },
        }
    }
}
impl Editor {
    pub(crate) fn change_collider_kind(
        &mut self,
        ids: &[EntityId],
        kind: ColliderKind,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        if ids.is_empty() {
            return Ok(());
        }
        let snapshot = self.document.expand(assets)?;
        let mut seen = BTreeSet::new();
        let mut edits = Vec::new();
        for &id in ids {
            ensure!(seen.insert(id), "碰撞形状编辑目标重复：{id}");
            let target = self.resolve_field_target(id, assets)?;
            let entity = snapshot
                .entities
                .binary_search_by_key(&id, |e| e.id)
                .ok()
                .map(|index| &snapshot.entities[index])
                .context("碰撞形状对象已不存在")?;
            let body = entity.physics.as_ref().context("对象缺少刚体组件")?;
            if ColliderKind::of(&body.shape) != kind {
                edits.push(FieldEdit {
                    target,
                    field: schema::COLLIDER_SHAPE,
                    value: FieldValue::ColliderShape(kind.initial()),
                });
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        self.preview_document(assets, |document, assets| {
            document.preview_field_values(&edits, assets)
        })
    }
}
