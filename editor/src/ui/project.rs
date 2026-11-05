use crate::{documents::DocumentAction, session::Editor};

pub(super) fn menu(editor: &mut Editor, ui: &mut egui::Ui) {
    ui.menu_button("项目", |ui| {
        for (label, create) in [("新建项目", true), ("打开项目", false)] {
            if ui.button(label).clicked() {
                let current = editor.project_root().to_owned();
                editor.documents.open_project_browser(create, &current);
                ui.close();
            }
        }
        ui.menu_button("最近项目", |ui| {
            let entries = editor.recent_projects.entries().to_vec();
            if entries.is_empty() {
                ui.weak(if editor.recent_projects.is_enabled() {
                    "还没有最近项目。"
                } else {
                    "当前运行未启用最近项目记录。"
                });
            }
            for entry in entries {
                ui.push_id(&entry.root, |ui| {
                    ui.horizontal(|ui| {
                        let name = entry.root.file_name().unwrap_or(entry.root.as_os_str());
                        if ui
                            .button(name.to_string_lossy())
                            .on_hover_text(format!(
                                "{}\n作者文件：{}",
                                entry.root.display(),
                                entry.source.display()
                            ))
                            .clicked()
                        {
                            editor.request_document_action(DocumentAction::OpenProject {
                                root: entry.root.clone(),
                                scene: entry.source.clone(),
                            });
                            ui.close();
                        }
                        if ui.small_button("移除").clicked() {
                            editor.remove_recent_project(&entry.root);
                        }
                    });
                });
            }
        });
        if let Some(warning) = &editor.recent_warning {
            ui.separator();
            ui.colored_label(egui::Color32::LIGHT_YELLOW, "最近项目记录未更新");
            ui.collapsing("记录诊断", |ui| {
                ui.set_max_width(420.);
                ui.label(warning);
            });
            if ui.small_button("关闭记录提示").clicked() {
                editor.recent_warning = None;
            }
        }
        ui.separator();
        if ui.button("项目设置").clicked() {
            editor.project.open = true;
            ui.close();
        }
    });
}
