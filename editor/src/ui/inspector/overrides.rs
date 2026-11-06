use crate::session::Editor;
use comet_asset::AssetStore;
use comet_core::{EntityId, WorldId};
use comet_editor_core::FieldTarget;
use comet_scene::schema::{self, FieldKey};
use std::sync::Arc;

struct OverrideCache {
    world: WorldId,
    document_revision: u64,
    products_revision: String,
    selected: EntityId,
    target: Result<FieldTarget, String>,
    fields: Vec<FieldKey>,
}

impl OverrideCache {
    fn matches(&self, editor: &Editor, assets: &AssetStore, selected: EntityId) -> bool {
        self.world == editor.document.world_id()
            && self.document_revision == editor.document.revision()
            && self.products_revision == assets.revision()
            && self.selected == selected
    }

    fn read(editor: &Editor, assets: &mut AssetStore, selected: EntityId) -> Self {
        let target = editor
            .resolve_field_target(selected, assets)
            .map_err(|error| format!("{error:#}"));
        let fields = if let Ok(FieldTarget::Prefab { instance, entity }) = target {
            editor
                .document
                .shared_source()
                .instances
                .iter()
                .find(|value| value.id == instance)
                .into_iter()
                .flat_map(|value| &value.overrides)
                .filter(|value| value.entity == entity)
                .map(|value| value.field)
                .collect()
        } else {
            vec![]
        };
        Self {
            world: editor.document.world_id(),
            document_revision: editor.document.revision(),
            products_revision: assets.revision().to_owned(),
            selected,
            target,
            fields,
        }
    }
}

fn cached(
    editor: &Editor,
    ui: &egui::Ui,
    assets: &mut AssetStore,
    id: EntityId,
) -> Arc<OverrideCache> {
    // One replaceable entry avoids retaining products or accumulating state for every inspected entity.
    let cache_id = egui::Id::unique("inspector-prefab-overrides-cache");
    ui.data(|data| data.get_temp::<Arc<OverrideCache>>(cache_id))
        .filter(|cache| cache.matches(editor, assets, id))
        .unwrap_or_else(|| {
            let cache = Arc::new(OverrideCache::read(editor, assets, id));
            ui.data_mut(|data| data.insert_temp(cache_id, cache.clone()));
            cache
        })
}

pub(super) fn target(
    editor: &Editor,
    ui: &egui::Ui,
    assets: &mut AssetStore,
    id: EntityId,
) -> Result<FieldTarget, String> {
    cached(editor, ui, assets, id).target.clone()
}

pub(super) fn draw(editor: &mut Editor, ui: &mut egui::Ui, assets: &mut AssetStore, id: EntityId) {
    if editor.play.is_some() {
        return;
    }
    let cache = cached(editor, ui, assets, id);
    match &cache.target {
        Ok(FieldTarget::Authored(_)) => return,
        Err(error) => {
            ui.weak("嵌套实例的覆盖请在源 Prefab 中编辑。")
                .on_hover_text(error);
            return;
        }
        Ok(FieldTarget::Prefab { .. }) => {}
    }
    ui.separator();
    let editable = !editor.is_busy() && !editor.documents.is_blocking();
    let mut restore = None;
    egui::CollapsingHeader::new(format!("实例覆盖 ({})", cache.fields.len()))
        .id_salt("inspector-prefab-overrides")
        .default_open(false)
        .show(ui, |ui| {
            ui.weak("恢复继承会删除实例的显式覆盖，重新采用源 Prefab 的值。");
            if cache.fields.is_empty() {
                ui.weak("当前实体没有显式覆盖。");
            } else {
                for &field in &cache.fields {
                    ui.push_id((field.component, field.field), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(field_label(field));
                            if ui
                                .add_enabled(editable, egui::Button::new("恢复继承"))
                                .clicked()
                            {
                                restore = Some(vec![field]);
                            }
                        });
                    });
                }
                if ui
                    .add_enabled(editable, egui::Button::new("恢复全部继承"))
                    .clicked()
                {
                    restore = Some(cache.fields.clone());
                }
            }
        });
    if let Some(fields) = restore {
        let result = editor.revert_prefab_fields(&[id], Some(&fields), assets);
        editor.notice(result);
    }
}

fn field_label(field: FieldKey) -> &'static str {
    match field {
        schema::TRANSLATION => "变换 · 位置",
        schema::ROTATION => "变换 · 旋转",
        schema::SCALE => "变换 · 缩放",
        schema::MESH => "网格渲染 · 网格",
        schema::MATERIAL => "网格渲染 · 材质",
        schema::ROUGHNESS => "网格渲染 · 粗糙度",
        schema::TARGET => "引用 · 目标",
        schema::CONNECTIONS => "引用 · 连接列表",
        schema::MASS => "刚体 · 质量 (kg)",
        schema::FRICTION => "刚体 · 摩擦系数",
        schema::MOVE_SPEED => "角色控制器 · 移动速度 (m/s)",
        schema::JUMP_SPEED => "角色控制器 · 跳跃速度 (m/s)",
        schema::CROUCH_SPEED => "角色控制器 · 蹲伏速度 (m/s)",
        schema::CROUCH_HEIGHT_RATIO => "角色控制器 · 蹲伏高度比例",
        schema::SCRIPT_ASSET => "脚本 · 资源",
        schema::SCRIPT_ENABLED => "脚本 · 启用",
        schema::SCRIPT_PARAMETERS => "脚本 · 参数",
        schema::POINT_ENABLED => "点关节 · 启用",
        schema::POINT_CONNECTION => "点关节 · 连接目标",
        schema::POINT_ANCHOR => "点关节 · 本地锚点",
        schema::POINT_CONNECTED_ANCHOR => "点关节 · 连接锚点",
        schema::POINT_CONTACTS_ENABLED => "点关节 · 连接体碰撞",
        schema::HINGE_ENABLED => "铰链关节 · 启用",
        schema::HINGE_CONNECTION => "铰链关节 · 连接目标",
        schema::HINGE_ANCHOR => "铰链关节 · 本地锚点",
        schema::HINGE_CONNECTED_ANCHOR => "铰链关节 · 连接锚点",
        schema::HINGE_CONTACTS_ENABLED => "铰链关节 · 连接体碰撞",
        schema::HINGE_ROTATION => "铰链关节 · 本端朝向",
        schema::HINGE_CONNECTED_ROTATION => "铰链关节 · 连接端朝向",
        schema::HINGE_LIMITS => "铰链关节 · 限位",
        schema::HINGE_MOTOR => "铰链关节 · 驱动",
        schema::SLIDER_ENABLED => "滑动关节 · 启用",
        schema::SLIDER_CONNECTION => "滑动关节 · 连接目标",
        schema::SLIDER_ANCHOR => "滑动关节 · 本地锚点",
        schema::SLIDER_CONNECTED_ANCHOR => "滑动关节 · 连接锚点",
        schema::SLIDER_CONTACTS_ENABLED => "滑动关节 · 连接体碰撞",
        schema::SLIDER_ROTATION => "滑动关节 · 本端朝向",
        schema::SLIDER_CONNECTED_ROTATION => "滑动关节 · 连接端朝向",
        schema::SLIDER_LIMITS => "滑动关节 · 限位",
        schema::SLIDER_MOTOR => "滑动关节 · 驱动",
        schema::NAVIGATION_DATA => "导航区域 · 导航数据",
        schema::NAVIGATION_HALF_EXTENTS => "导航区域 · 半尺寸",
        schema::NAVIGATION_CELL_SIZE => "导航区域 · 网格尺寸",
        schema::NAVIGATION_AGENT_RADIUS => "导航区域 · 代理半径",
        schema::NAVIGATION_AGENT_HEIGHT => "导航区域 · 代理高度",
        schema::NAVIGATION_MAX_CLIMB => "导航区域 · 最大攀爬",
        schema::NAVIGATION_MAX_SLOPE => "导航区域 · 最大坡度",
        schema::NAVIGATION_ENABLED => "导航代理 · 启用",
        schema::NAVIGATION_TARGET => "导航代理 · 目标",
        schema::NAVIGATION_ARRIVAL_DISTANCE => "导航代理 · 到达距离",
        schema::AUDIO_CLIP => "音频源 · 音频片段",
        schema::AUDIO_GAIN => "音频源 · 音量",
        schema::AUDIO_LOOPING => "音频源 · 循环",
        schema::AUDIO_AUTOPLAY => "音频源 · 启动时播放",
        schema::AUDIO_SPATIAL => "音频源 · 空间音效",
        schema::AUDIO_MIN_DISTANCE => "音频源 · 开始衰减距离（米）",
        schema::AUDIO_MAX_DISTANCE => "音频源 · 最大距离（米）",
        schema::ANIMATOR_SKELETON => "动画 · 骨架",
        schema::ANIMATOR_CLIP => "动画 · 播放片段",
        schema::ANIMATOR_BINDINGS => "动画 · 节点绑定",
        schema::ANIMATOR_LOOPING => "动画 · 循环",
        schema::ANIMATOR_SPEED => "动画 · 播放速度",
        schema::SKIN_ASSET => "蒙皮绑定 · 蒙皮",
        schema::SKIN_ANIMATOR => "蒙皮绑定 · 动画播放器",
        _ => "其他 · 字段",
    }
}

struct GroupCache {
    world: WorldId,
    revision: u64,
    products: String,
    selection: u64,
    ids: Option<Vec<EntityId>>,
    fields: Vec<(FieldKey, usize)>,
}

pub(super) fn draw_group(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    assets: &mut AssetStore,
    entities: &[&comet_scene::EntityData],
) {
    if editor.play.is_some() {
        return;
    }
    let cache_id = egui::Id::unique("inspector-group-overrides-cache");
    let cache = ui
        .data(|data| data.get_temp::<Arc<GroupCache>>(cache_id))
        .filter(|cache| {
            cache.world == editor.document.world_id()
                && cache.revision == editor.document.revision()
                && cache.products == assets.revision()
                && cache.selection == editor.selection.generation()
        })
        .unwrap_or_else(|| {
            let mut ids = Vec::new();
            let mut fields = std::collections::BTreeMap::new();
            let mut eligible = true;
            for entity in entities {
                let Ok(FieldTarget::Prefab {
                    instance,
                    entity: source_entity,
                }) = editor.resolve_field_target(entity.id, assets)
                else {
                    eligible = false;
                    break;
                };
                ids.push(entity.id);
                if let Some(instance) = editor
                    .document
                    .shared_source()
                    .instances
                    .iter()
                    .find(|value| value.id == instance)
                {
                    for value in instance
                        .overrides
                        .iter()
                        .filter(|value| value.entity == source_entity)
                    {
                        *fields
                            .entry((value.field.component, value.field.field))
                            .or_insert(0) += 1;
                    }
                }
            }
            let cache = Arc::new(GroupCache {
                world: editor.document.world_id(),
                revision: editor.document.revision(),
                products: assets.revision().into(),
                selection: editor.selection.generation(),
                ids: eligible.then_some(ids),
                fields: fields
                    .into_iter()
                    .map(|((component, field), count)| (FieldKey { component, field }, count))
                    .collect(),
            });
            ui.data_mut(|data| data.insert_temp(cache_id, cache.clone()));
            cache
        });
    let Some(ids) = &cache.ids else {
        return;
    };
    let editable = !editor.is_busy() && !editor.documents.is_blocking() && ui.is_enabled();
    let mut restore = None;
    ui.separator();
    egui::CollapsingHeader::new(format!("实例覆盖 ({})", cache.fields.len()))
        .id_salt((
            "inspector-group-overrides",
            editor.world(),
            editor.selection.generation(),
        ))
        .show(ui, |ui| {
            ui.weak("恢复继承会删除所选实体的显式覆盖，分别采用源 Prefab 的值。");
            if cache.fields.is_empty() {
                ui.weak("当前选择没有显式覆盖。");
            }
            for &(field, count) in &cache.fields {
                ui.push_id((field.component, field.field), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} ({}/{})", field_label(field), count, ids.len()));
                        if ui
                            .add_enabled(editable, egui::Button::new("恢复继承"))
                            .clicked()
                        {
                            restore = Some(Some(field));
                        }
                    })
                });
            }
            if !cache.fields.is_empty()
                && ui
                    .add_enabled(editable, egui::Button::new("恢复全部继承"))
                    .clicked()
            {
                restore = Some(None);
            }
        });
    if let Some(field) = restore {
        let fields = field.map(|field| [field]);
        let result = editor.revert_prefab_fields(
            ids,
            fields.as_ref().map(|fields| fields.as_slice()),
            assets,
        );
        editor.notice(result);
    }
}

#[cfg(test)]
mod tests;
