use crate::documents::{BrowserMode, DocumentAction, is_authoring_source};
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;

mod data;

enum ResourceAction {
    Place(comet_asset_pipeline::SourceAsset),
    EditSource(std::path::PathBuf),
}

pub(super) fn draw(editor: &mut Editor, ui: &mut egui::Ui, assets: &mut AssetStore) {
    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !editor.is_busy() && !editor.documents.is_blocking(),
                egui::Button::new("新建 Prefab"),
            )
            .clicked()
        {
            editor.documents.open_browser(BrowserMode::NewPrefab);
        }
        if ui
            .add_enabled(
                editor.play.is_none() && !editor.is_busy(),
                egui::Button::new("刷新资源"),
            )
            .clicked()
        {
            let result = editor.refresh_resources();
            editor.notice(result);
        }
        let selected = editor
            .resource_selection
            .and_then(|id| {
                editor
                    .sources
                    .of_kind(AssetKind::Prefab)
                    .find(|choice| choice.id == id)
            })
            .cloned();
        let is_self = selected
            .as_ref()
            .is_some_and(|source| Some(source.id) == editor.document.prefab_asset());
        if ui
            .add_enabled(
                editor.play.is_none() && !editor.is_busy() && selected.is_some() && !is_self,
                egui::Button::new(format!("添加到{}", editor.document_kind_label())),
            )
            .on_disabled_hover_text(if is_self {
                "不能将 Prefab 自身添加为实例。"
            } else {
                "请选择资源，并在编辑模式中添加。"
            })
            .clicked()
        {
            action = selected.clone().map(ResourceAction::Place);
        }
        if ui
            .add_enabled(
                !editor.is_busy()
                    && !editor.documents.is_blocking()
                    && selected.as_ref().is_some_and(is_authoring_source),
                egui::Button::new("编辑源"),
            )
            .on_disabled_hover_text("请选择 .prefab 文件；glTF/GLB 请在建模工具中修改。")
            .clicked()
        {
            action = selected
                .as_ref()
                .map(|source| ResourceAction::EditSource(source.source.clone()));
        }
        ui.weak("模型、Prefab 与数据");
    });
    if editor.resource_build.is_running() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(&editor.resource_build.stage);
        });
    }
    if editor.navigation.is_running() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(&editor.navigation.stage);
        });
    }
    if let Some(error) = &editor.navigation.failure {
        ui.colored_label(egui::Color32::LIGHT_RED, "导航烘焙未完成");
        ui.collapsing("导航烘焙诊断", |ui| {
            ui.label(error);
        });
    }
    if let Some(error) = &editor.resource_build.failure {
        ui.colored_label(egui::Color32::LIGHT_RED, "资源刷新未完成，当前内容仍保留。");
        ui.collapsing("刷新诊断详情", |ui| {
            ui.label(error);
        });
    }
    egui::ScrollArea::vertical()
        .id_salt("resources-list")
        .show(ui, |ui| {
            for choice in editor
                .sources
                .of_kind(AssetKind::Prefab)
                .chain(editor.sources.of_kind(AssetKind::Data))
            {
                let response = ui.selectable_label(
                    editor.resource_selection == Some(choice.id),
                    format!(
                        "{}  {}",
                        if choice.kind == AssetKind::Data {
                            "数据"
                        } else if is_authoring_source(choice) {
                            "Prefab"
                        } else {
                            "模型"
                        },
                        choice.source.display()
                    ),
                );
                if response.clicked() {
                    editor.resource_selection = Some(choice.id);
                }
                if response.double_clicked()
                    && choice.kind == AssetKind::Prefab
                    && editor.play.is_none()
                    && !editor.is_busy()
                    && Some(choice.id) != editor.document.prefab_asset()
                {
                    action = Some(ResourceAction::Place(choice.clone()));
                }
            }
            if editor
                .sources
                .of_kind(AssetKind::Prefab)
                .chain(editor.sources.of_kind(AssetKind::Data))
                .next()
                .is_none()
            {
                ui.weak("项目中没有已构建的模型、Prefab 或数据。");
            }
        });
    match action {
        Some(ResourceAction::Place(choice)) => {
            let result = editor.place_resource(
                choice.id,
                &choice
                    .source
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy(),
                assets,
            );
            editor.notice(result);
        }
        Some(ResourceAction::EditSource(path)) => {
            editor.request_document_action(DocumentAction::Open(path))
        }
        None => {}
    }
}

pub(super) fn console(editor: &mut Editor, ui: &mut egui::Ui) {
    if ui.button("清空").clicked() {
        editor.console.clear();
    }
    egui::ScrollArea::vertical()
        .id_salt("script-log")
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for entry in editor.console.entries() {
                let color = if entry.failed {
                    egui::Color32::LIGHT_RED
                } else {
                    egui::Color32::LIGHT_GRAY
                };
                ui.colored_label(
                    color,
                    format!(
                        "{} · tick {} · {}",
                        if entry.failed { "错误" } else { "日志" },
                        entry.tick,
                        entry.phase
                    ),
                );
                ui.small(format!("实体：{} · 脚本：{}", entry.entity, entry.asset))
                    .on_hover_text(format!(
                        "运行世界：{} · 序号：{}",
                        entry.world, entry.sequence
                    ));
                ui.label(&entry.message);
                ui.separator();
            }
            if editor.console.entries().is_empty() {
                ui.weak("当前没有脚本日志。运行场景后，脚本消息和错误显示在这里。");
            }
        });
}

pub(super) fn preview(editor: &mut Editor, ctx: &egui::Context, assets: &mut AssetStore) {
    data::show(editor, ctx, assets);
}
