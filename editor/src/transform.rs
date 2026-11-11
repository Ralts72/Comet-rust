use crate::session::Editor;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_core::{
    EntityId, WorldId,
    math::{Mat3, Mat4, Quat, Vec3},
};
use comet_editor_core::{FieldEdit, FieldTarget};
use comet_scene::{
    FieldUpdate, Transform, checked_transform_inverse,
    schema::{self, FieldKey, FieldValue},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum GizmoTool {
    #[default]
    Translate,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TransformSpace {
    #[default]
    World,
    Local,
}

#[derive(Clone, Copy, Debug)]
enum Destination {
    Document(FieldTarget),
    Play(EntityId),
}

#[derive(Debug)]
struct TransformTarget {
    destination: Destination,
    is_active: bool,
    local: Transform,
    world: Mat4,
    parent_inverse: Mat4,
}

#[derive(Debug)]
pub(crate) struct TransformDrag {
    pub anchor: Vec3,
    pub axes: Option<[Vec3; 3]>,
    tool: GizmoTool,
    space: Option<TransformSpace>,
    world: WorldId,
    selection_generation: u64,
    products_revision: String,
    targets: Vec<TransformTarget>,
    translation_preview: Vec3,
    rotation_preview: Option<(Vec3, f32)>,
    scale_basis: Option<(Mat4, Mat4)>,
    scale_preview: Vec3,
}

impl Editor {
    pub(crate) fn begin_translation(&mut self, assets: &mut AssetStore) -> Result<()> {
        self.begin_transform(GizmoTool::Translate, assets)
    }

    pub(crate) fn begin_rotation(&mut self, assets: &mut AssetStore) -> Result<()> {
        self.begin_transform(GizmoTool::Rotate, assets)
    }

    pub(crate) fn begin_scale(&mut self, assets: &mut AssetStore) -> Result<()> {
        self.begin_transform(GizmoTool::Scale, assets)
    }

    fn begin_transform(&mut self, tool: GizmoTool, assets: &mut AssetStore) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再变换对象");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        ensure!(
            self.play.is_none() || self.selection.len() == 1,
            "运行期间暂不支持成组变换"
        );
        let active = self.selection.active().context("请先选择需要变换的对象")?;
        let snapshot = if let Some(play) = &mut self.play {
            play.snapshot()
        } else {
            self.document.expand(assets)?
        };
        let transforms = comet_scene::world_transforms(&snapshot)?;
        let entities: BTreeMap<_, _> = snapshot
            .entities
            .iter()
            .map(|entity| (entity.id, entity))
            .collect();
        let anchor = transforms
            .get(&active)
            .context("主对象已失效")?
            .transform_point3(Vec3::ZERO);
        let mut targets = Vec::new();
        for id in self.selection.ids() {
            let entity = entities.get(&id).context("所选对象已失效")?;
            let mut parent = entity.parent;
            let mut covered = false;
            while let Some(id) = parent {
                if self.selection.contains(id) {
                    covered = true;
                    break;
                }
                parent = entities.get(&id).context("父对象已失效")?.parent;
            }
            if covered {
                continue;
            }
            let parent = entity.parent.map_or(Mat4::IDENTITY, |id| transforms[&id]);
            let parent_inverse = checked_transform_inverse(parent)
                .context("父对象变换不可逆，无法保持世界方向变换")?;
            let destination = if self.play.is_some() {
                ensure!(entity.physics.is_none(), "运行期间请通过物理接口变换对象");
                Destination::Play(id)
            } else {
                Destination::Document(self.resolve_field_target(id, assets)?)
            };
            targets.push(TransformTarget {
                destination,
                is_active: id == active,
                local: entity.transform.clone(),
                world: transforms[&id],
                parent_inverse,
            });
        }
        ensure!(!targets.is_empty(), "没有可变换的对象");
        let active_entity = entities[&active];
        let scale_basis = if tool == GizmoTool::Scale {
            let parent = active_entity
                .parent
                .map_or(Mat4::IDENTITY, |id| transforms[&id]);
            scale_basis(parent, Quat::from_array(active_entity.transform.rotation))
                .and_then(|basis| Ok((basis, checked_transform_inverse(basis)?)))
                .ok()
        } else {
            None
        };
        let axes = if tool == GizmoTool::Scale {
            scale_basis.map(|(basis, _)| {
                [
                    basis.x_axis.truncate(),
                    basis.y_axis.truncate(),
                    basis.z_axis.truncate(),
                ]
            })
        } else {
            Some(rigid_axes(&snapshot, active, self.transform_space)?)
        };
        let drag = TransformDrag {
            anchor,
            axes,
            tool,
            space: (tool != GizmoTool::Scale).then_some(self.transform_space),
            world: self.world(),
            selection_generation: self.selection.generation(),
            products_revision: assets.revision().to_owned(),
            targets,
            translation_preview: Vec3::ZERO,
            rotation_preview: Some((Vec3::ZERO, 0.)),
            scale_basis,
            scale_preview: Vec3::ONE,
        };
        self.commit()?;
        if self.play.is_none() {
            self.document.begin()?;
        }
        self.picking.clear();
        self.drag_origin = Some(drag);
        Ok(())
    }

    fn transform_drag(&self, tool: GizmoTool, assets: &AssetStore) -> Result<&TransformDrag> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再变换对象");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        let drag = self.drag_origin.as_ref().context("没有活动的变换操作")?;
        ensure!(drag.tool == tool, "变换工具已变化，请重新拖动");
        ensure!(
            drag.space.is_none_or(|space| space == self.transform_space),
            "变换坐标系已变化，请重新拖动"
        );
        ensure!(
            drag.world == self.world() && drag.selection_generation == self.selection.generation(),
            "选择或场景已变化，请重新拖动"
        );
        ensure!(
            drag.products_revision == assets.revision(),
            "资源已更新，请重新拖动"
        );
        Ok(drag)
    }

    fn preview_transform_fields(
        &mut self,
        tool: GizmoTool,
        assets: &mut AssetStore,
        fields: impl Fn(&TransformTarget, Vec3) -> Result<Vec<(FieldKey, FieldValue)>>,
    ) -> Result<()> {
        let drag = self.transform_drag(tool, assets)?;
        let mut edits = Vec::new();
        let mut runtime_edits = Vec::new();
        for target in &drag.targets {
            for (field, mut value) in fields(target, drag.anchor)? {
                match target.destination {
                    Destination::Document(destination) => {
                        if transform_fields(&target.local)
                            .into_iter()
                            .any(|(key, baseline)| key == field && baseline == value)
                        {
                            continue;
                        }
                        self.source_translation_value(destination, field, &mut value, assets)?;
                        edits.push(FieldEdit {
                            target: destination,
                            field,
                            value,
                        });
                    }
                    Destination::Play(entity) => runtime_edits.push(FieldUpdate {
                        entity,
                        field,
                        value,
                    }),
                }
            }
        }
        if let Some(play) = &mut self.play {
            play.set_fields(&runtime_edits)
        } else {
            self.document.preview_fields_from_baseline(&edits, assets)
        }
    }

    pub(crate) fn preview_translation(
        &mut self,
        world_delta: Vec3,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(world_delta.is_finite(), "移动超出有效范围");
        if self
            .transform_drag(GizmoTool::Translate, assets)?
            .translation_preview
            == world_delta
        {
            return Ok(());
        }
        self.preview_transform_fields(GizmoTool::Translate, assets, |target, _| {
            let value = Vec3::from_array(target.local.translation)
                + target.parent_inverse.transform_vector3(world_delta);
            ensure!(value.is_finite(), "移动超出有效范围");
            Ok(vec![(
                schema::TRANSLATION,
                FieldValue::Vec3(value.to_array()),
            )])
        })?;
        self.drag_origin
            .as_mut()
            .context("变换操作已失效")?
            .translation_preview = world_delta;
        Ok(())
    }

    pub(crate) fn preview_rotation(
        &mut self,
        axis: Vec3,
        angle_radians: f32,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(
            axis.is_finite() && (axis.length_squared() - 1.).abs() <= 1e-4,
            "旋转轴必须是有限单位世界方向"
        );
        ensure!(angle_radians.is_finite(), "旋转角度超出有效范围");
        let drag = self.transform_drag(GizmoTool::Rotate, assets)?;
        let turns = (angle_radians / std::f32::consts::TAU).round();
        let identity = angle_radians == 0.
            || (turns != 0.
                && (angle_radians - turns * std::f32::consts::TAU).abs()
                    <= 2. * f32::EPSILON * angle_radians.abs());
        let accepted = (axis, if identity { 0. } else { angle_radians });
        if drag.rotation_preview == Some(accepted)
            || (identity && drag.rotation_preview.is_some_and(|(_, angle)| angle == 0.))
        {
            return Ok(());
        }
        let rotation =
            Quat::from_axis_angle(axis.normalize(), angle_radians % std::f32::consts::TAU);
        self.preview_transform_fields(GizmoTool::Rotate, assets, |target, pivot| {
            let mut local = if identity {
                target.local.clone()
            } else {
                let delta = Mat4::from_translation(pivot)
                    * Mat4::from_quat(rotation)
                    * Mat4::from_translation(-pivot);
                stable_transform(target.parent_inverse * delta * target.world, &target.local)
                    .context("此旋转需要剪切或奇异变换，无法保持对象形状")?
            };
            if target.world.transform_point3(Vec3::ZERO) == pivot {
                local.translation = target.local.translation;
            }
            Ok(transform_fields(&local))
        })?;
        self.drag_origin
            .as_mut()
            .context("变换操作已失效")?
            .rotation_preview = Some(accepted);
        Ok(())
    }

    pub(crate) fn preview_scale(&mut self, factors: Vec3, assets: &mut AssetStore) -> Result<()> {
        ensure!(
            factors.is_finite() && factors.min_element() > 0.,
            "缩放倍率必须是有限正数"
        );
        let drag = self.transform_drag(GizmoTool::Scale, assets)?;
        if drag.scale_preview == factors {
            return Ok(());
        }
        let uniform = factors.x == factors.y && factors.x == factors.z;
        let delta = if uniform {
            Mat4::IDENTITY
        } else {
            let (basis, inverse) = drag
                .scale_basis
                .context("主对象本地缩放轴不可逆，请使用统一缩放")?;
            Mat4::from_translation(drag.anchor)
                * basis
                * Mat4::from_scale(factors)
                * inverse
                * Mat4::from_translation(-drag.anchor)
        };
        self.preview_transform_fields(GizmoTool::Scale, assets, |target, pivot| {
            let mut local = if factors == Vec3::ONE {
                target.local.clone()
            } else if uniform || target.is_active {
                let mut local = target.local.clone();
                local.scale = (Vec3::from_array(local.scale) * factors).to_array();
                if uniform {
                    let world_origin = target.world.transform_point3(Vec3::ZERO);
                    let displacement = (world_origin - pivot) * (factors.x - 1.);
                    local.translation = (Vec3::from_array(local.translation)
                        + target.parent_inverse.transform_vector3(displacement))
                    .to_array();
                }
                local
            } else {
                stable_transform(target.parent_inverse * delta * target.world, &target.local)
                    .context("整组轴向缩放需要剪切或奇异变换，请使用统一缩放")?
            };
            if target.world.transform_point3(Vec3::ZERO) == pivot {
                local.translation = target.local.translation;
            }
            ensure!(
                local.translation.into_iter().all(f32::is_finite),
                "缩放位置超出有效范围"
            );
            for (original, scaled) in target.local.scale.into_iter().zip(&local.scale) {
                ensure!(
                    scaled.is_finite()
                        && (original == 0.
                            || (*scaled != 0.
                                && original.is_sign_negative() == scaled.is_sign_negative())),
                    "缩放超出有效范围，不能溢出或跨过零轴"
                );
            }
            Ok(transform_fields(&local))
        })?;
        self.drag_origin
            .as_mut()
            .context("变换操作已失效")?
            .scale_preview = factors;
        Ok(())
    }

    pub(crate) fn cancel_transform(&mut self) -> Result<()> {
        if let Some(drag) = &self.drag_origin
            && drag.world == self.world()
            && let Some(play) = &mut self.play
        {
            let mut updates = Vec::new();
            for target in &drag.targets {
                if let Destination::Play(entity) = target.destination {
                    for (field, value) in transform_fields(&target.local) {
                        updates.push(FieldUpdate {
                            entity,
                            field,
                            value,
                        });
                    }
                }
            }
            play.set_fields(&updates)?;
        }
        self.cancel()
    }
}

/// Rigid tool axes follow authored rotations, excluding scale and reflection.
/// The local frame chooses group directions; every root receives the same world transform.
pub(crate) fn rigid_axes(
    snapshot: &comet_scene::SceneSnapshot,
    primary: EntityId,
    space: TransformSpace,
) -> Result<[Vec3; 3]> {
    let find = |id| {
        // Published snapshots are sorted; tolerate unsorted caller-owned snapshots as well.
        snapshot
            .entities
            .binary_search_by_key(&id, |entity| entity.id)
            .ok()
            .map(|index| &snapshot.entities[index])
            .or_else(|| snapshot.entities.iter().find(|entity| entity.id == id))
            .context("坐标轴所属对象已失效")
    };
    let mut entity = find(primary)?;
    if space == TransformSpace::World {
        return Ok([Vec3::X, Vec3::Y, Vec3::Z]);
    }
    let mut rotation = Quat::IDENTITY;
    let mut depth = 0;
    loop {
        depth += 1;
        ensure!(depth <= snapshot.entities.len(), "坐标轴的父级形成循环");
        let local = Quat::from_array(entity.transform.rotation);
        ensure!(
            local.is_finite() && (local.length_squared() - 1.).abs() < 0.001,
            "坐标轴需要有效的单位四元数"
        );
        rotation = (local.normalize() * rotation).normalize();
        let Some(parent) = entity.parent else { break };
        entity = find(parent)?;
    }
    Ok([rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z])
}

/// Local authored axes after the parent affine transform, without the object's own scale.
/// Columns need not be orthogonal: replacing this frame with a rotation changes local scaling.
pub(crate) fn scale_basis(parent_world: Mat4, local_rotation: Quat) -> Result<Mat4> {
    ensure!(
        parent_world.is_finite()
            && local_rotation.is_finite()
            && (local_rotation.length_squared() - 1.).abs() < 0.001,
        "无效的本地缩放坐标"
    );
    checked_transform_inverse(parent_world)?;
    let matrix = parent_world * Mat4::from_quat(local_rotation);
    let axes = [
        matrix.x_axis.truncate(),
        matrix.y_axis.truncate(),
        matrix.z_axis.truncate(),
    ]
    .map(|axis| axis.try_normalize().context("本地缩放轴不可用"));
    let [x, y, z] = axes;
    let basis = Mat4::from_cols(
        x?.extend(0.),
        y?.extend(0.),
        z?.extend(0.),
        Vec3::ZERO.extend(1.),
    );
    checked_transform_inverse(basis)?;
    Ok(basis)
}

fn stable_transform(matrix: Mat4, baseline: &Transform) -> Result<Transform> {
    let decomposed = Transform::from_matrix(matrix)?;
    let mut stable = decomposed.clone();
    let close =
        |left: f32, right: f32| (left - right).abs() <= 4. * f32::EPSILON * right.abs().max(1.);
    let relative = |left: f32, right: f32| {
        (left - right).abs() <= 4. * f32::EPSILON * left.abs().max(right.abs())
    };
    // Keep the authored reflection axes; equivalent decompositions must not pin inherited scale.
    for axis in 0..3 {
        let scale = decomposed.scale[axis].abs().copysign(baseline.scale[axis]);
        stable.scale[axis] = if relative(scale, baseline.scale[axis]) {
            baseline.scale[axis]
        } else {
            scale
        };
        if relative(stable.translation[axis], baseline.translation[axis]) {
            stable.translation[axis] = baseline.translation[axis];
        }
    }
    let rotation = Quat::from_mat3(&Mat3::from_cols(
        matrix.x_axis.truncate() / stable.scale[0],
        matrix.y_axis.truncate() / stable.scale[1],
        matrix.z_axis.truncate() / stable.scale[2],
    ))
    .normalize();
    let baseline_rotation = Quat::from_array(baseline.rotation);
    let rotation = if rotation.dot(baseline_rotation) < 0. {
        -rotation
    } else {
        rotation
    };
    stable.rotation = rotation.to_array();
    if stable
        .rotation
        .into_iter()
        .zip(baseline.rotation)
        .all(|(a, b)| close(a, b))
    {
        stable.rotation = baseline.rotation;
    }
    if stable
        .matrix()
        .to_cols_array()
        .into_iter()
        .zip(matrix.to_cols_array())
        .all(|(a, b)| a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-5 * b.abs().max(1.))
    {
        Ok(stable)
    } else {
        Ok(decomposed)
    }
}

fn transform_fields(transform: &Transform) -> Vec<(FieldKey, FieldValue)> {
    vec![
        (schema::TRANSLATION, FieldValue::Vec3(transform.translation)),
        (schema::ROTATION, FieldValue::Quat(transform.rotation)),
        (schema::SCALE, FieldValue::Vec3(transform.scale)),
    ]
}

#[cfg(test)]
mod tests;
