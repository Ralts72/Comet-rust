use crate::session::Editor;
use anyhow::{Context, Result};
use comet_asset::AssetStore;
use comet_core::EntityId;
use comet_editor_core::JointEdit;
use comet_scene::schema::{self, FieldKey, FieldValue};
use comet_scene::{BodyMotion, EntityData};

impl Editor {
    pub(crate) fn edit_joint(
        &mut self,
        id: EntityId,
        mut edit: JointEdit,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let target = self.resolve_field_target(id, assets)?;
        let settings = match &mut edit {
            JointEdit::Point(value) => value.as_mut().map(|joint| &mut joint.settings),
            JointEdit::Hinge(value) => value.as_mut().map(|joint| &mut joint.settings),
            JointEdit::Slider(value) => value.as_mut().map(|joint| &mut joint.settings),
        };
        if let Some(settings) = settings {
            let mut value = FieldValue::JointConnection(settings.connection.clone());
            self.source_field_value(target, &mut value, assets)?;
            let FieldValue::JointConnection(connection) = value else {
                unreachable!()
            };
            settings.connection = connection;
        }
        self.preview_document(assets, |document, assets| {
            document.preview_joint(id, edit, assets)
        })
        .context("无法更新关节，请检查刚体、连接目标、锚点和限制参数")
    }

    pub(crate) fn edit_joint_fields(
        &mut self,
        before: &EntityData,
        after: &EntityData,
        fields: &[FieldKey],
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let mut changed = Vec::new();
        for key in fields {
            let field = schema::descriptor(*key)?;
            let original = field.read(before);
            let next = field.read(after).context("关节组件已不存在")?;
            if original != Some(next.clone()) {
                changed.push((*key, next));
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        let target = self.resolve_field_target(before.id, assets)?;
        for (_, value) in &mut changed {
            self.source_field_value(target, value, assets)?;
        }
        let edits: Vec<_> = changed
            .into_iter()
            .map(|(field, value)| comet_editor_core::FieldEdit {
                target,
                field,
                value,
            })
            .collect();
        self.preview_document(assets, |document, assets| {
            document.preview_field_values(&edits, assets)
        })
    }

    pub(crate) fn joint_targets(
        &self,
        owner: &EntityData,
        ctx: &egui::Context,
        assets: &mut AssetStore,
    ) -> Result<Vec<(EntityId, String)>> {
        let allowed = self.cached_field_references(ctx, owner.id, assets)?;
        let owner_dynamic = owner
            .physics
            .as_ref()
            .is_some_and(|body| body.motion == BodyMotion::Dynamic);
        Ok(allowed
            .targets
            .iter()
            .filter_map(|(id, name)| {
                let entity = self
                    .last_snapshot
                    .entities
                    .binary_search_by_key(id, |entity| entity.id)
                    .ok()
                    .map(|index| &self.last_snapshot.entities[index])?;
                let valid = entity.id != owner.id
                    && entity.character.is_none()
                    && entity
                        .physics
                        .as_ref()
                        .is_some_and(|body| owner_dynamic || body.motion == BodyMotion::Dynamic);
                valid.then(|| (*id, name.clone()))
            })
            .collect())
    }
}
