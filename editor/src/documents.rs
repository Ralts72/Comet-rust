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
mod tests {
    use super::*;
    use comet_application::Application;
    use comet_scene::{schema, schema::FieldValue};

    fn preview(editor: &mut Editor, assets: &mut AssetStore) -> anyhow::Result<()> {
        let id = editor
            .snapshot(assets)?
            .entities
            .iter()
            .find(|entity| entity.name == "Amber cube")
            .unwrap()
            .id;
        editor.edit(
            id,
            schema::TRANSLATION,
            FieldValue::Vec3([2., 3., 4.]),
            assets,
        )
    }

    #[test]
    fn cancel_preserves_a_live_preview_and_discard_never_writes_it() -> anyhow::Result<()> {
        let (_temp, mut editor, mut assets) = crate::session::tests::fixture()?;
        let saved = std::fs::read(&editor.source)?;
        preview(&mut editor, &mut assets)?;
        let edited = editor.document.shared_source();
        assert!(!editor.request_close());
        assert!(
            !editor.request_close(),
            "repeated close must not bypass confirmation"
        );
        editor.resolve_transition(Choice::Cancel, &mut assets);
        assert!(!editor.close_ready());
        assert!(editor.document.is_editing());
        assert_eq!(editor.document.shared_source(), edited);
        assert!(!editor.request_close());
        editor.resolve_transition(Choice::Discard, &mut assets);
        assert!(editor.close_ready());
        assert_eq!(std::fs::read(&editor.source)?, saved);
        Ok(())
    }

    #[test]
    fn save_failure_stays_open_and_retry_persists_authoring_after_play() -> anyhow::Result<()> {
        let (_temp, mut editor, mut assets) = crate::session::tests::fixture()?;
        preview(&mut editor, &mut assets)?;
        editor.start(&mut assets)?;
        let authoring = editor.document.shared_source();
        let source = editor.source.clone();
        let backup = source.with_extension("backup");
        std::fs::rename(&source, &backup)?;
        std::fs::create_dir(&source)?;
        assert!(!editor.request_close());
        editor.resolve_transition(Choice::Save, &mut assets);
        assert!(!editor.close_ready());
        assert!(editor.documents.pending && editor.documents.error.is_some());
        assert!(editor.document.is_dirty());
        assert!(editor.play.is_none());
        assert_eq!(editor.document.shared_source(), authoring);
        std::fs::remove_dir(&source)?;
        std::fs::rename(&backup, &source)?;
        editor.resolve_transition(Choice::Save, &mut assets);
        assert!(editor.close_ready());
        assert!(!editor.document.is_dirty());
        let reopened = comet_editor_core::Document::load(&source)?;
        assert_eq!(reopened.shared_source(), authoring);
        Ok(())
    }

    #[test]
    fn unchanged_authoring_closes_directly_during_play() -> anyhow::Result<()> {
        let (_temp, mut editor, mut assets) = crate::session::tests::fixture()?;
        editor.start(&mut assets)?;
        assert!(editor.request_close());
        assert!(editor.close_ready());
        Ok(())
    }

    #[test]
    fn scene_creation_can_be_cancelled_and_switching_saves_the_current_authoring()
    -> anyhow::Result<()> {
        let (_temp, mut editor, mut assets) = crate::session::tests::fixture()?;
        let original = editor.source.clone();
        editor.export.template_error = Some("preserve template configuration".into());
        preview(&mut editor, &mut assets)?;
        let edited = editor.document.shared_source();
        editor.request_document_action(DocumentAction::New("new.scene".into()));
        editor.resolve_transition(Choice::Cancel, &mut assets);
        assert!(editor.document.is_editing());
        assert!(!original.parent().unwrap().join("new.scene").exists());
        editor.request_document_action(DocumentAction::New("new.scene".into()));
        editor.resolve_transition(Choice::Save, &mut assets);
        assert!(editor.documents.error.is_none());
        assert!(editor.source.ends_with("new.scene"));
        assert_eq!(editor.document.history_len(), 0);
        assert!(!editor.document.is_dirty());
        assert!(editor.document.source().base.entities.is_empty());
        assert_eq!(
            comet_editor_core::Document::load(&original)?.shared_source(),
            edited
        );
        let metadata: comet_asset_pipeline::SourceMeta = serde_json::from_slice(&std::fs::read(
            original.parent().unwrap().join("new.scene.meta"),
        )?)?;
        assert_eq!(editor.export.entry_scene(), metadata.id);
        assert_eq!(editor.export.choices.len(), 2);
        assert_eq!(
            editor.export.template_error.as_deref(),
            Some("preserve template configuration")
        );
        editor.request_document_action(DocumentAction::Open(original.clone()));
        editor.resolve_transition(Choice::Discard, &mut assets);
        assert_eq!(editor.source, original);
        assert_eq!(editor.document.shared_source(), edited);
        assert_eq!(editor.document.history_len(), 0);
        Ok(())
    }

    #[test]
    fn failed_open_keeps_current_play_document_and_product_store() -> anyhow::Result<()> {
        let (_temp, mut editor, mut assets) = crate::session::tests::fixture()?;
        preview(&mut editor, &mut assets)?;
        editor.start(&mut assets)?;
        let source = editor.source.clone();
        let content = editor.document.shared_source();
        let world = editor.world();
        let revision = assets.revision().to_owned();
        let bad = source.parent().unwrap().join("broken.scene");
        std::fs::write(&bad, b"not a scene")?;
        for path in [bad, "../outside.scene".into()] {
            editor.request_document_action(DocumentAction::Open(path));
            editor.resolve_transition(Choice::Discard, &mut assets);
            assert!(editor.documents.error.is_some());
            assert_eq!(editor.source, source);
            assert_eq!(editor.world(), world);
            assert_eq!(editor.document.shared_source(), content);
            assert_eq!(assets.revision(), revision);
            assert!(editor.document.is_dirty());
            editor.resolve_transition(Choice::Cancel, &mut assets);
        }
        assert!(!editor.close_ready());
        Ok(())
    }

    #[test]
    fn save_failure_prevents_a_new_scene_from_being_created() -> anyhow::Result<()> {
        let (_temp, mut editor, mut assets) = crate::session::tests::fixture()?;
        preview(&mut editor, &mut assets)?;
        let source = editor.source.clone();
        let backup = source.with_extension("backup");
        std::fs::rename(&source, &backup)?;
        std::fs::create_dir(&source)?;
        editor.request_document_action(DocumentAction::New("new.scene".into()));
        editor.resolve_transition(Choice::Save, &mut assets);
        assert!(editor.documents.error.is_some());
        assert!(editor.document.is_dirty());
        assert_eq!(editor.source, source);
        assert!(!source.parent().unwrap().join("new.scene").exists());
        assert!(!source.parent().unwrap().join("new.scene.meta").exists());
        Ok(())
    }
}

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
        choices: &[crate::scenes::SceneChoice],
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
                        for scene in choices {
                            if ui
                                .selectable_label(
                                    self.browser.selected.as_ref() == Some(&scene.source),
                                    &scene.label,
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
