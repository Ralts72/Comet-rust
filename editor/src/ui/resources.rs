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
                        editor.play.is_none() && selected.is_some(),
                        egui::Button::new("添加到场景"),
                    )
                    .clicked()
                {
                    place = selected;
                }
                ui.weak("模型与 Prefab · 新增或修改资源后重新启动编辑器");
            });
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
                        if response.double_clicked() && editor.play.is_none() {
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
