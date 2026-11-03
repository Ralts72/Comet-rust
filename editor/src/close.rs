use crate::session::Editor;
use comet_asset::AssetStore;

#[derive(Default)]
pub(crate) struct CloseState {
    pub pending: bool,
    pub approved: bool,
    error: Option<String>,
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
        editor.resolve_close(Choice::Cancel, &mut assets);
        assert!(!editor.close_ready());
        assert!(editor.document.is_editing());
        assert_eq!(editor.document.shared_source(), edited);
        assert!(!editor.request_close());
        editor.resolve_close(Choice::Discard, &mut assets);
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
        editor.resolve_close(Choice::Save, &mut assets);
        assert!(!editor.close_ready());
        assert!(editor.close.pending && editor.close.error.is_some());
        assert!(editor.document.is_dirty());
        assert!(editor.play.is_none());
        assert_eq!(editor.document.shared_source(), authoring);
        std::fs::remove_dir(&source)?;
        std::fs::rename(&backup, &source)?;
        editor.resolve_close(Choice::Save, &mut assets);
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
}

#[derive(Clone, Copy)]
enum Choice {
    Save,
    Discard,
    Cancel,
}

impl Editor {
    pub(crate) fn request_editor_close(&mut self) -> bool {
        if self.close.approved {
            return true;
        }
        if !self.document.is_dirty() && !self.export.is_running() {
            self.close.approved = true;
            return true;
        }
        self.close.pending = true;
        false
    }

    fn resolve_close(&mut self, choice: Choice, assets: &mut AssetStore) {
        if matches!(choice, Choice::Cancel) {
            self.close = CloseState::default();
            return;
        }
        // A running export owns a candidate package and must finish before the process exits.
        if self.export.is_running() {
            return;
        }
        match choice {
            Choice::Save => {
                self.stop();
                match self.save(assets) {
                    Ok(()) => self.close.approved = true,
                    Err(error) => self.close.error = Some(format!("{error:#}")),
                }
            }
            Choice::Discard => self.close.approved = true,
            Choice::Cancel => unreachable!(),
        }
    }

    pub(crate) fn show_close(&mut self, ctx: &egui::Context, assets: &mut AssetStore) {
        if !self.close.pending || self.close.approved {
            return;
        }
        let busy = self.export.is_running();
        if !busy && !self.document.is_dirty() {
            self.close.approved = true;
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("close-editor")).show(ctx, |ui| {
            ui.set_width(420.);
            ui.heading("退出编辑器");
            ui.separator();
            if busy {
                ui.label("正在导出游戏，请等待任务完成后退出。");
            } else {
                ui.label("当前场景有未保存的修改，是否保存后退出？");
                if self.play.is_some() {
                    ui.weak("保存将停止运行并保存作者场景。");
                }
            }
            if let Some(error) = &self.close.error {
                ui.colored_label(egui::Color32::LIGHT_RED, "保存失败，编辑器保持打开。");
                ui.label(error);
            }
            ui.add_space(12.);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("保存并退出"))
                    .clicked()
                {
                    choice = Some(Choice::Save);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("直接退出"))
                    .clicked()
                {
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
            self.resolve_close(choice, assets);
        }
    }
}
