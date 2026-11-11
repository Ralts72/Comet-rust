use crate::documents::{BrowserMode, DocumentAction, is_authoring_source};
use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use std::path::PathBuf;

mod browser;
mod data;
mod drag;
mod tree;
pub(super) use drag::{ResourceDrag, cancel_drag, finish_drag};

enum ResourceAction {
    Place(comet_asset_pipeline::SourceAsset),
    EditSource(PathBuf),
}

fn can_place(editor: &Editor, source: &comet_asset_pipeline::SourceAsset) -> bool {
    source.kind == AssetKind::Prefab
        && editor.play.is_none()
        && !editor.is_busy()
        && !editor.documents.is_blocking()
        && Some(source.id) != editor.document.prefab_asset()
}
fn place(
    editor: &mut Editor,
    source: &comet_asset_pipeline::SourceAsset,
    assets: &mut AssetStore,
) -> anyhow::Result<()> {
    editor.place_resource(
        source.id,
        &source
            .source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy(),
        assets,
    )
}

pub(super) fn draw(editor: &mut Editor, ui: &mut egui::Ui, assets: &mut AssetStore) {
    let mut browser = browser::Browser::load(editor, ui, assets);
    let mut action = None;
    egui::ScrollArea::horizontal()
        .id_salt("resource-toolbar")
        .auto_shrink([false, true])
        .show(ui, |ui| {
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
                        editor.play.is_none()
                            && !editor.is_busy()
                            && !editor.documents.is_blocking(),
                        egui::Button::new("刷新资源"),
                    )
                    .clicked()
                {
                    let result = editor.refresh_resources();
                    editor.notice(result);
                }
                let selected = editor
                    .resource_selection
                    .and_then(|id| editor.sources.iter().find(|source| source.id == id))
                    .cloned();
                if ui
                    .add_enabled(
                        selected
                            .as_ref()
                            .is_some_and(|source| can_place(editor, source)),
                        egui::Button::new(format!("添加到{}", editor.document_kind_label())),
                    )
                    .on_disabled_hover_text(
                        "请选择模型或 Prefab，在编辑模式中添加；不能添加 Prefab 自身。",
                    )
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
                    .on_disabled_hover_text(
                        "请选择场景或 .prefab 文件；glTF/GLB 请在建模工具中修改。",
                    )
                    .clicked()
                {
                    action = selected
                        .as_ref()
                        .map(|source| ResourceAction::EditSource(source.source.clone()));
                }
                browser.controls(ui);
            });
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
    browser.show(editor, ui, assets, &mut action);
    match action {
        Some(ResourceAction::Place(source)) => {
            let result = place(editor, &source, assets);
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
