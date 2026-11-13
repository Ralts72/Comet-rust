use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_core::{
    EntityId,
    math::{Quat, Vec3},
};
use comet_editor_core::JointEdit;
use comet_runtime::JointKind;
use comet_scene::schema::{self, FieldKey, FieldValue};
use comet_scene::{
    BodyMotion, EntityData, EntityRef, HingeJoint, JointConnection, JointFrame, JointSettings,
    PointJoint, SceneSnapshot, SliderJoint,
};

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

    /// Reconnect the complete selection using each owner's current frame and source scope.
    pub(crate) fn replace_joint_connections(
        &mut self,
        ids: &[EntityId],
        field: FieldKey,
        connection: JointConnection,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        ensure!(
            matches!(
                field,
                schema::POINT_CONNECTION | schema::HINGE_CONNECTION | schema::SLIDER_CONNECTION
            ),
            "此字段不是关节连接"
        );
        ensure!(
            !matches!(&connection, JointConnection::Body(reference) if !matches!(reference, EntityRef::Local { .. })),
            "请选择展开后的连接刚体"
        );
        let snapshot = self.document.expand(assets)?;
        let mut unique = std::collections::BTreeSet::new();
        let mut edits = Vec::new();
        for &id in ids {
            ensure!(unique.insert(id), "关节目标重复：{id}");
            let target = self.resolve_field_target(id, assets)?;
            let owner = entity(&snapshot, id)?;
            let descriptor = schema::descriptor(field)?;
            let before = descriptor.read(owner).context("对象缺少关节组件")?;
            if let JointConnection::Body(EntityRef::Local { entity: id }) = &connection {
                ensure!(
                    self.field_references(target, assets)?
                        .targets
                        .iter()
                        .any(|(target, _)| target == id),
                    "连接目标不属于全部对象允许的直接作用域"
                );
                ensure!(
                    crate::fields::accepts_binding(
                        field,
                        owner,
                        entity(&snapshot, *id)?,
                        &snapshot,
                        assets.index()
                    ),
                    "连接目标需要另一根非角色刚体，且至少一端为动态刚体"
                );
            }
            if matches!(connection, JointConnection::World) {
                ensure!(
                    owner
                        .physics
                        .as_ref()
                        .is_some_and(|body| body.motion == BodyMotion::Dynamic),
                    "世界连接需要自身为动态刚体"
                );
            }
            let mut value = FieldValue::JointConnection(connection.clone());
            self.source_field_value(target, &mut value, assets)?;
            if before == FieldValue::JointConnection(connection.clone()) {
                continue;
            }
            edits.push(comet_editor_core::FieldEdit {
                target,
                field,
                value,
            });
            if matches!(connection, JointConnection::Unbound) {
                continue;
            }
            let (anchor, connected_anchor, orientations) = match field {
                schema::POINT_CONNECTION => {
                    (schema::POINT_ANCHOR, schema::POINT_CONNECTED_ANCHOR, None)
                }
                schema::HINGE_CONNECTION => (
                    schema::HINGE_ANCHOR,
                    schema::HINGE_CONNECTED_ANCHOR,
                    Some((schema::HINGE_ROTATION, schema::HINGE_CONNECTED_ROTATION)),
                ),
                schema::SLIDER_CONNECTION => (
                    schema::SLIDER_ANCHOR,
                    schema::SLIDER_CONNECTED_ANCHOR,
                    Some((schema::SLIDER_ROTATION, schema::SLIDER_CONNECTED_ROTATION)),
                ),
                _ => unreachable!(),
            };
            let FieldValue::Vec3(local) = schema::descriptor(anchor)?
                .read(owner)
                .context("缺少关节锚点")?
            else {
                unreachable!()
            };
            let owner_rotation = Quat::from_array(owner.transform.rotation).normalize();
            let world = Vec3::from_array(owner.transform.translation)
                + owner_rotation
                    * (Vec3::from_array(owner.transform.scale) * Vec3::from_array(local));
            let destination = match &connection {
                JointConnection::Body(EntityRef::Local { entity: id }) => {
                    Some(entity(&snapshot, *id)?)
                }
                _ => None,
            };
            let rotation =
                destination.map(|target| Quat::from_array(target.transform.rotation).normalize());
            let point = if let Some(target) = destination {
                rotation.unwrap().inverse()
                    * (world - Vec3::from_array(target.transform.translation))
                    / Vec3::from_array(target.transform.scale)
            } else {
                world
            };
            edits.push(comet_editor_core::FieldEdit {
                target,
                field: connected_anchor,
                value: FieldValue::Vec3(point.to_array()),
            });
            if let Some((local_rotation, connected_rotation)) = orientations {
                let FieldValue::Quat(local) = schema::descriptor(local_rotation)?
                    .read(owner)
                    .context("缺少关节朝向")?
                else {
                    unreachable!()
                };
                let world = owner_rotation * Quat::from_array(local).normalize();
                let orientation = rotation.map_or(world, |rotation| rotation.inverse() * world);
                edits.push(comet_editor_core::FieldEdit {
                    target,
                    field: connected_rotation,
                    value: FieldValue::Quat(orientation.to_array()),
                });
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        self.preview_document(assets, |document, assets| {
            document.preview_field_values(&edits, assets)
        })
        .context("无法更新关节连接，已保留整组原配置")
    }
}

fn entity(snapshot: &SceneSnapshot, id: EntityId) -> Result<&EntityData> {
    snapshot
        .entities
        .binary_search_by_key(&id, |entity| entity.id)
        .ok()
        .map(|index| &snapshot.entities[index])
        .context("关节对象或连接目标已失效")
}

pub(crate) fn initial_joint(kind: JointKind, entity: &EntityData) -> JointEdit {
    let connected_frame = JointFrame {
        anchor: entity.transform.translation,
        rotation: entity.transform.rotation,
    };
    let settings = JointSettings {
        connection: if entity
            .physics
            .as_ref()
            .is_some_and(|body| body.motion == BodyMotion::Dynamic)
        {
            JointConnection::World
        } else {
            JointConnection::Unbound
        },
        ..Default::default()
    };
    match kind {
        JointKind::Point => JointEdit::Point(Some(PointJoint {
            settings,
            connected_anchor: entity.transform.translation,
            ..Default::default()
        })),
        JointKind::Hinge => JointEdit::Hinge(Some(HingeJoint {
            settings,
            connected_frame,
            ..Default::default()
        })),
        JointKind::Slider => JointEdit::Slider(Some(SliderJoint {
            settings,
            connected_frame,
            ..Default::default()
        })),
    }
}
