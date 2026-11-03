use crate::session::Editor;
use comet_asset::AssetStore;

#[derive(Default)]
pub(crate) struct DocumentWorkflow {
    pub pending: bool,
    pub approved: bool,
    error: Option<String>,
    action: Option<DocumentAction>,
    browser: SceneBrowser,
}

#[cfg(test)]
#[path = "../tests/unit/documents.rs"]
mod tests;

#[derive(Clone, Copy)]
enum Choice {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone)]
pub(crate) enum DocumentAction {
    Exit,
    Open(std::path::PathBuf),
    New(std::path::PathBuf),
}
#[derive(Default)]
struct SceneBrowser {
    create: Option<bool>,
    path: String,
    selected: Option<std::path::PathBuf>,
}
impl DocumentWorkflow {
    pub fn is_blocking(&self) -> bool {
        self.pending || self.browser.create.is_some()
    }
    pub fn open_browser(&mut self, create: bool) {
        self.browser.create = Some(create);
        self.browser.selected = None;
        self.browser.path = if create {
            "new.scene".into()
        } else {
            String::new()
        };
    }
    pub fn show_browser(
        &mut self,
        ctx: &egui::Context,
        sources: &comet_asset_pipeline::SourceCatalog,
    ) -> Option<DocumentAction> {
        let create = self.browser.create?;
        let mut action = None;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("scene-browser")).show(ctx, |ui| {
            ui.set_width(480.);
            ui.heading(if create {
                "新建场景"
            } else {
                "打开场景"
            });
            if create {
                ui.label("场景位置（相对项目 assets，使用 .scene 后缀）");
                ui.text_edit_singleline(&mut self.browser.path);
                ui.weak("创建后立即保存空场景及其身份文件，不覆盖已有文件。");
            } else {
                ui.label("选择项目中的作者场景");
                egui::ScrollArea::vertical()
                    .max_height(280.)
                    .show(ui, |ui| {
                        for scene in sources.of_kind(comet_asset_data::product::AssetKind::Scene) {
                            if ui
                                .selectable_label(
                                    self.browser.selected.as_ref() == Some(&scene.source),
                                    scene.source.display().to_string(),
                                )
                                .clicked()
                            {
                                self.browser.selected = Some(scene.source.clone());
                            }
                        }
                    });
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        if create {
                            !self.browser.path.trim().is_empty()
                        } else {
                            self.browser.selected.is_some()
                        },
                        egui::Button::new(if create { "创建" } else { "打开" }),
                    )
                    .clicked()
                {
                    action = if create {
                        Some(DocumentAction::New(self.browser.path.trim().into()))
                    } else {
                        self.browser.selected.clone().map(DocumentAction::Open)
                    };
                }
                if ui.button("取消").clicked() {
                    cancel = true;
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        if action.is_some() || cancel {
            self.browser.create = None;
        }
        action
    }
}
impl Editor {
    pub(crate) fn request_document_action(&mut self, action: DocumentAction) {
        self.documents.action = Some(action);
        self.documents.pending = true;
        self.documents.error = None;
    }
    pub(crate) fn request_editor_close(&mut self) -> bool {
        if self.documents.approved {
            return true;
        }
        if !self.document.is_dirty() && !self.export.is_running() {
            self.documents.approved = true;
            return true;
        }
        self.documents.action = Some(DocumentAction::Exit);
        self.documents.pending = true;
        self.documents.browser.create = None;
        false
    }
    fn finish_transition(&mut self, assets: &mut AssetStore) -> anyhow::Result<()> {
        match self
            .documents
            .action
            .clone()
            .unwrap_or(DocumentAction::Exit)
        {
            DocumentAction::Exit => self.documents.approved = true,
            DocumentAction::Open(path) => {
                self.switch_scene(path, assets)?;
                self.documents = DocumentWorkflow::default();
            }
            DocumentAction::New(path) => {
                let created = self.create_scene_source(&path)?;
                // If loading fails, retry the complete source already created without recreating its ID.
                self.documents.action = Some(DocumentAction::Open(created.clone()));
                self.switch_scene(created, assets)?;
                self.documents = DocumentWorkflow::default();
            }
        }
        Ok(())
    }
    fn resolve_transition(&mut self, choice: Choice, assets: &mut AssetStore) {
        if matches!(choice, Choice::Cancel) {
            self.documents = DocumentWorkflow::default();
            return;
        }
        if self.export.is_running() {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            if matches!(choice, Choice::Save) {
                self.stop();
                self.save(assets)?;
            }
            self.finish_transition(assets)
        })();
        if let Err(error) = result {
            self.documents.error = Some(format!("{error:#}"));
        }
    }
    pub(crate) fn show_document_confirmation(
        &mut self,
        ctx: &egui::Context,
        assets: &mut AssetStore,
    ) {
        if !self.documents.pending || self.documents.approved {
            return;
        }
        let busy = self.export.is_running();
        if !busy && !self.document.is_dirty() && self.documents.error.is_none() {
            self.resolve_transition(Choice::Discard, assets);
            if !self.documents.pending || self.documents.approved {
                return;
            }
        }
        let exit = matches!(self.documents.action, Some(DocumentAction::Exit) | None);
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("document-confirmation")).show(ctx, |ui| {
            ui.set_width(480.);
            ui.heading(if exit {
                "退出编辑器"
            } else {
                "切换场景"
            });
            ui.separator();
            if busy {
                ui.label("正在导出游戏，请等待任务完成。");
            } else if self.document.is_dirty() {
                ui.label("当前场景有未保存的修改，是否保存后继续？");
                if self.play.is_some() {
                    ui.weak("保存将停止运行并保存作者场景。");
                }
            }
            if let Some(error) = &self.documents.error {
                ui.colored_label(egui::Color32::LIGHT_RED, "操作未完成，当前场景仍保留。");
                ui.label(error);
            }
            ui.add_space(12.);
            ui.horizontal(|ui| {
                if self.document.is_dirty() {
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(if exit {
                                "保存并退出"
                            } else {
                                "保存并继续"
                            }),
                        )
                        .clicked()
                    {
                        choice = Some(Choice::Save);
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(if exit {
                                "直接退出"
                            } else {
                                "放弃修改并继续"
                            }),
                        )
                        .clicked()
                    {
                        choice = Some(Choice::Discard);
                    }
                } else if ui.add_enabled(!busy, egui::Button::new("重试")).clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.button("取消").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                choice = Some(Choice::Cancel);
            }
        });
        if let Some(choice) = choice {
            self.resolve_transition(choice, assets);
        }
    }
}
