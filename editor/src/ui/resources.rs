use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;

pub(super) fn show(editor: &mut Editor, root: &mut egui::Ui, assets: &mut AssetStore) {
    egui::Panel::bottom("resources")
        .default_size(145.)
        .min_size(95.)
        .resizable(true)
        .show(root, |ui| {
            let mut place = None;
            ui.horizontal(|ui| {
                ui.heading("资源");
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
                if ui
                    .add_enabled(
                        editor.play.is_none() && !editor.is_busy() && selected.is_some(),
                        egui::Button::new("添加到场景"),
                    )
                    .clicked()
                {
                    place = selected;
                }
                ui.weak("模型与 Prefab");
            });
            if editor.resource_build.is_running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&editor.resource_build.stage);
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
                    for choice in editor.sources.of_kind(AssetKind::Prefab) {
                        let response = ui.selectable_label(
                            editor.resource_selection == Some(choice.id),
                            format!(
                                "{}  {}",
                                if choice.source.extension().is_some_and(|e| e == "prefab") {
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
                        if response.double_clicked() && editor.play.is_none() && !editor.is_busy() {
                            place = Some(choice.clone());
                        }
                    }
                    if editor.sources.of_kind(AssetKind::Prefab).next().is_none() {
                        ui.weak("项目中没有已构建的模型或 Prefab。");
                    }
                });
            if let Some(choice) = place {
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
        });
}
