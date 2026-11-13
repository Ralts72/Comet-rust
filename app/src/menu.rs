use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum MenuAction {
    Resume,
    Save,
    Continue,
    Apply(Settings),
    Exit,
}

pub(super) struct MenuFrame {
    pub blocked: bool,
    pub changed: bool,
    pub action: Option<MenuAction>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Settings,
}

struct VolumeEdit {
    before: f32,
    widget: egui::Id,
}

#[derive(Default)]
pub(super) struct Menu {
    page: Option<Page>,
    draft: Settings,
    edit: Option<VolumeEdit>,
    error: Option<String>,
    escape_held: bool,
    escape_blocked: bool,
    gamepad_pause: comet_application::GamepadPauseInput,
    composing: bool,
    pointer_blocked: bool,
}

impl Menu {
    pub(super) fn is_open(&self) -> bool {
        self.page.is_some()
    }

    pub(super) fn close(&mut self) -> bool {
        let open = self.is_open();
        self.page = None;
        self.edit = None;
        self.error = None;
        self.composing = false;
        open
    }

    pub(super) fn feedback(&mut self, error: Option<String>) {
        self.error = error;
    }

    /// Called before simulation. A closing frame still belongs to the menu.
    pub(super) fn show(
        &mut self,
        viewport: &egui::Response,
        accepted: &Settings,
        active: bool,
        activity_changed: bool,
        gamepad: &comet_application::GamepadFrame,
    ) -> MenuFrame {
        let ctx = &viewport.ctx;
        let was_open = self.is_open();
        let active = active && ctx.input(|input| input.focused);
        let (escape, ime_captured) = self.escape(viewport, active, activity_changed);
        let pause = self.gamepad_pause.sample(
            ctx,
            gamepad,
            active
                && !activity_changed
                && !ime_captured
                && (self.is_open() || comet_application::input_authorized(viewport, true)),
        );
        let escape = escape || pause;
        if !active || activity_changed {
            self.cancel_edit(ctx);
            self.pointer_blocked = true;
        }
        let pointer_blocked = self.pointer_blocked;
        if !ctx.input(|input| input.pointer.any_down()) {
            self.pointer_blocked = false;
        }
        if !was_open && escape && !ime_captured {
            self.page = Some(Page::Main);
        }
        let mut action = None;
        if let Some(page) = self.page {
            egui::Modal::new(egui::Id::unique("game-menu")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.).clamp(1., 360.));
                ui.heading(if page == Page::Main {
                    "游戏已暂停"
                } else {
                    "设置"
                });
                ui.separator();
                let height: f32 = if page == Page::Main { 260. } else { 200. };
                egui::ScrollArea::vertical()
                    // Reserve feedback space without growing a remembered Modal rect.
                    .max_height(height.min((ctx.content_rect().height() - 140.).max(1.)))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(error) = &self.error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                            ui.add_space(8.);
                        }
                        ui.add_enabled_ui(active && !activity_changed && !pointer_blocked, |ui| {
                            match page {
                                Page::Main => self.main_page(ui, accepted, &mut action),
                                Page::Settings => self.settings_page(
                                    ui,
                                    accepted,
                                    escape || ime_captured,
                                    &mut action,
                                ),
                            }
                        });
                    });
            });
            // The opening press cannot also close the newly displayed menu.
            if was_open && escape && !ime_captured {
                action = None;
                if page == Page::Settings {
                    if self.edit.is_some() {
                        self.cancel_edit(ctx);
                    } else {
                        self.page = Some(Page::Main);
                        self.draft = *accepted;
                        self.error = None;
                    }
                } else {
                    self.close();
                    action = Some(MenuAction::Resume);
                }
            }
        }
        let open = self.is_open();
        MenuFrame {
            blocked: was_open || open,
            changed: was_open != open,
            action,
        }
    }

    fn main_page(
        &mut self,
        ui: &mut egui::Ui,
        accepted: &Settings,
        action: &mut Option<MenuAction>,
    ) {
        for (label, command) in [
            ("返回游戏", MenuAction::Resume),
            ("保存进度", MenuAction::Save),
            ("继续存档", MenuAction::Continue),
        ] {
            if button(ui, label) {
                *action = Some(command);
                self.error = None;
                if command == MenuAction::Resume {
                    self.close();
                }
            }
        }
        if button(ui, "设置") {
            self.page = Some(Page::Settings);
            self.draft = *accepted;
            self.edit = None;
            self.error = None;
        }
        if button(ui, "退出游戏") {
            *action = Some(MenuAction::Exit);
        }
        ui.add_space(8.);
        ui.weak("继续存档将从保存关卡入口开始。");
    }

    fn settings_page(
        &mut self,
        ui: &mut egui::Ui,
        accepted: &Settings,
        cancelling: bool,
        action: &mut Option<MenuAction>,
    ) {
        let before = self.draft.master_volume;
        let response = ui.add(
            egui::Slider::new(&mut self.draft.master_volume, 0. ..=1.)
                .text("主音量")
                .fixed_decimals(2),
        );
        if self.edit.is_none() && (response.drag_started() || response.has_focus()) {
            self.edit = Some(VolumeEdit {
                before,
                widget: response.id,
            });
        }
        if !cancelling && (response.drag_stopped() || response.lost_focus()) {
            self.edit = None;
        }
        ui.checkbox(&mut self.draft.shadows, "方向光阴影");
        ui.add_space(8.);
        ui.weak("应用后保存设置；返回将放弃尚未应用的修改。");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.draft != *accepted, egui::Button::new("应用"))
                .clicked()
            {
                self.edit = None;
                self.error = None;
                *action = Some(MenuAction::Apply(self.draft));
            }
            if ui.button("返回").clicked() {
                self.page = Some(Page::Main);
                self.draft = *accepted;
                self.edit = None;
                self.error = None;
            }
        });
    }

    fn cancel_edit(&mut self, ctx: &egui::Context) {
        if let Some(edit) = self.edit.take() {
            self.draft.master_volume = edit.before;
            ctx.memory_mut(|memory| memory.surrender_focus(edit.widget));
            ctx.stop_dragging();
            self.pointer_blocked = ctx.input(|input| input.pointer.any_down());
        }
    }

    fn escape(
        &mut self,
        viewport: &egui::Response,
        active: bool,
        activity_changed: bool,
    ) -> (bool, bool) {
        let ctx = &viewport.ctx;
        let authorized = active
            && !activity_changed
            && (self.is_open() || viewport.has_focus() || !ctx.egui_wants_keyboard_input());
        ctx.input(|input| {
            let mut ime_captured = self.composing;
            for event in &input.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                        self.composing = !text.is_empty();
                        ime_captured |= self.composing;
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(_)) => self.composing = false,
                    _ => {}
                }
            }
            if !active || activity_changed {
                self.composing = false;
            }
            if !authorized {
                self.escape_held = input.key_down(egui::Key::Escape);
                self.escape_blocked = self.escape_held;
                return (false, ime_captured);
            }
            let mut escape = false;
            for event in &input.events {
                if let egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed,
                    repeat,
                    ..
                } = event
                {
                    if *pressed {
                        escape |= !repeat && !self.escape_held && !self.escape_blocked;
                        self.escape_held = true;
                    } else {
                        self.escape_held = false;
                        self.escape_blocked = false;
                    }
                }
            }
            if !input.key_down(egui::Key::Escape) {
                self.escape_held = false;
                self.escape_blocked = false;
            }
            (escape, ime_captured)
        })
    }
}

fn button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add_sized([ui.available_width(), 32.], egui::Button::new(label))
        .clicked()
}

#[cfg(test)]
mod tests;
