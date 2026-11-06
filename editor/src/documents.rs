use crate::session::Editor;
use comet_asset::AssetStore;
use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::SourceAsset;

pub(crate) fn is_authoring_source(source: &SourceAsset) -> bool {
    match source.kind {
        AssetKind::Scene => source.source.extension().is_some_and(|ext| ext == "scene"),
        AssetKind::Prefab => source.source.extension().is_some_and(|ext| ext == "prefab"),
        _ => false,
    }
}

#[derive(Default)]
pub(crate) struct DocumentWorkflow {
    pub pending: bool,
    pub approved: bool,
    error: Option<String>,
    action: Option<DocumentAction>,
    browser: DocumentBrowser,
    project_browser: crate::project::ProjectBrowser,
}

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
enum Choice {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DocumentAction {
    Exit,
    Open(std::path::PathBuf),
    NewScene(std::path::PathBuf),
    NewPrefab(std::path::PathBuf),
    OpenProject {
        root: std::path::PathBuf,
        scene: std::path::PathBuf,
    },
    NewProject(std::path::PathBuf),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserMode {
    Open,
    NewScene,
    NewPrefab,
}
#[derive(Default)]
struct DocumentBrowser {
    mode: Option<BrowserMode>,
    path: String,
    selected: Option<std::path::PathBuf>,
}
impl DocumentWorkflow {
    pub fn is_blocking(&self) -> bool {
        self.pending || self.browser.mode.is_some() || self.project_browser.is_open()
    }
    pub fn open_browser(&mut self, mode: BrowserMode) {
        self.project_browser = Default::default();
        self.browser.mode = Some(mode);
        self.browser.selected = None;
        self.browser.path = match mode {
            BrowserMode::Open => String::new(),
            BrowserMode::NewScene => "new.scene".into(),
            BrowserMode::NewPrefab => "new.prefab".into(),
        };
    }
    pub fn open_project_browser(&mut self, create: bool, current: &std::path::Path) {
        self.browser = Default::default();
        self.project_browser.open(create, current);
    }
    pub(super) fn matches_action(&self, action: &DocumentAction) -> bool {
        self.pending && self.action.as_ref() == Some(action)
    }
    pub(super) fn fail(&mut self, error: String) {
        self.error = Some(error);
    }
    pub fn show_browser(
        &mut self,
        ctx: &egui::Context,
        sources: &comet_asset_pipeline::SourceCatalog,
        reader: &mut crate::project::DirectoryReader,
    ) -> Option<DocumentAction> {
        if self.project_browser.is_open() {
            return self.project_browser.show(ctx, reader);
        }
        let mode = self.browser.mode?;
        let create = mode != BrowserMode::Open;
        let mut action = None;
        let mut cancel = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        egui::Modal::new(egui::Id::unique("document-browser")).show(ctx, |ui| {
            ui.set_width(480.);
            ui.heading(match mode {
                BrowserMode::Open => "打开场景 / Prefab",
                BrowserMode::NewScene => "新建场景",
                BrowserMode::NewPrefab => "新建 Prefab",
            });
            if create {
                ui.label(if mode == BrowserMode::NewPrefab {
                    "Prefab 位置（相对项目 assets，使用 .prefab 后缀）"
                } else {
                    "场景位置（相对项目 assets，使用 .scene 后缀）"
                });
                ui.text_edit_singleline(&mut self.browser.path);
                ui.weak("创建后立即保存，不覆盖已有文件。");
            } else {
                ui.label("选择项目中的场景或 Prefab 文件");
                egui::ScrollArea::vertical()
                    .max_height(280.)
                    .show(ui, |ui| {
                        for source in sources
                            .of_kind(AssetKind::Scene)
                            .chain(sources.of_kind(AssetKind::Prefab))
                            .filter(|source| is_authoring_source(source))
                        {
                            if ui
                                .selectable_label(
                                    self.browser.selected.as_ref() == Some(&source.source),
                                    format!(
                                        "{}  {}",
                                        if source.kind == AssetKind::Scene {
                                            "场景"
                                        } else {
                                            "Prefab"
                                        },
                                        source.source.display()
                                    ),
                                )
                                .clicked()
                            {
                                self.browser.selected = Some(source.source.clone());
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
                    action = match mode {
                        BrowserMode::Open => {
                            self.browser.selected.clone().map(DocumentAction::Open)
                        }
                        BrowserMode::NewScene => {
                            Some(DocumentAction::NewScene(self.browser.path.trim().into()))
                        }
                        BrowserMode::NewPrefab => {
                            Some(DocumentAction::NewPrefab(self.browser.path.trim().into()))
                        }
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
            self.browser.mode = None;
        }
        action
    }
}
impl Editor {
    pub(crate) fn document_kind_label(&self) -> &'static str {
        if self.document.prefab_asset().is_some() {
            "Prefab"
        } else {
            "场景"
        }
    }
    pub(crate) fn request_document_action(&mut self, action: DocumentAction) {
        self.documents.action = Some(action);
        self.documents.pending = true;
        self.documents.error = None;
    }
    pub(crate) fn request_editor_close(&mut self) -> bool {
        if self.documents.approved {
            return true;
        }
        if !self.document.is_dirty() && !self.is_busy() {
            self.documents.approved = true;
            return true;
        }
        self.documents.action = Some(DocumentAction::Exit);
        self.documents.pending = true;
        self.documents.browser.mode = None;
        self.documents.project_browser = Default::default();
        false
    }
    fn finish_transition(&mut self, assets: &mut AssetStore) -> anyhow::Result<()> {
        let path = match self
            .documents
            .action
            .clone()
            .unwrap_or(DocumentAction::Exit)
        {
            DocumentAction::Exit => {
                self.stop();
                self.documents.approved = true;
                return Ok(());
            }
            DocumentAction::Open(path) => path,
            DocumentAction::NewScene(path) => self.create_scene_source(&path)?,
            DocumentAction::NewPrefab(path) => self.create_prefab_source(&path)?,
            action @ (DocumentAction::NewProject(_) | DocumentAction::OpenProject { .. }) => {
                self.begin_project_transition(action)?;
                return Ok(());
            }
        };
        // A failed load retries the published source, preserving its identity and file contents.
        self.documents.action = Some(DocumentAction::Open(path.clone()));
        self.switch_document(path, assets)?;
        self.documents = DocumentWorkflow::default();
        Ok(())
    }
    fn resolve_transition(&mut self, choice: Choice, assets: &mut AssetStore) {
        if matches!(choice, Choice::Cancel) {
            self.documents = DocumentWorkflow::default();
            return;
        }
        if self.is_busy() {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            self.documents.error = None;
            if matches!(choice, Choice::Save) {
                self.save_authoring(assets)?;
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
        let mut busy = self.is_busy();
        if !busy && !self.document.is_dirty() && self.documents.error.is_none() {
            self.resolve_transition(Choice::Discard, assets);
            if !self.documents.pending || self.documents.approved {
                return;
            }
            busy = self.is_busy();
        }
        let exit = matches!(self.documents.action, Some(DocumentAction::Exit) | None);
        let project = matches!(
            self.documents.action,
            Some(DocumentAction::NewProject(_) | DocumentAction::OpenProject { .. })
        );
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("document-confirmation")).show(ctx, |ui| {
            ui.set_width(480.);
            ui.heading(if exit {
                "退出编辑器"
            } else if project {
                "切换项目"
            } else {
                "切换文档"
            });
            ui.separator();
            if busy {
                if self.project_load.is_running() {
                    ui.label(&self.project_load.stage);
                    ui.weak("可取消切换；后台准备会完成，已创建的项目会保留。");
                } else {
                    ui.label("正在处理后台任务，请等待完成。");
                }
            } else if self.document.is_dirty() {
                ui.label(format!(
                    "当前{}「{}」有未保存的修改，是否保存后继续？",
                    self.document_kind_label(),
                    self.source
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                if self.play.is_some() {
                    ui.weak(format!(
                        "保存作者{}，切换成功或确认退出后结束当前运行。",
                        self.document_kind_label()
                    ));
                }
            }
            if let Some(error) = &self.documents.error {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("操作未完成，当前{}仍保留。", self.document_kind_label()),
                );
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
