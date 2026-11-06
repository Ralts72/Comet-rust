use super::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};

struct Harness {
    ctx: egui::Context,
    menu: Menu,
    settings: Settings,
    texts: Vec<(String, egui::Rect)>,
    text_focus: bool,
    external: String,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        Self {
            ctx,
            menu: Menu::default(),
            settings: Settings::default(),
            texts: vec![],
            text_focus: false,
            external: String::new(),
        }
    }

    fn frame(&mut self, events: Vec<Event>, active: bool, changed: bool) -> MenuFrame {
        let ctx = self.ctx.clone();
        let mut result = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                focused: active,
                ..Default::default()
            },
            |ui| {
                if self.text_focus {
                    ui.text_edit_singleline(&mut self.external).request_focus();
                }
                let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click_and_drag());
                if response.clicked() {
                    response.request_focus();
                }
                result = Some(self.menu.show(&response, &self.settings, active, changed));
            },
        );
        self.texts.clear();
        for clipped in &output.shapes {
            text_shapes(&clipped.shape, &mut self.texts);
        }
        output.textures_delta.clear();
        result.unwrap()
    }

    fn settle(&mut self) {
        self.frame(vec![], true, false);
        self.frame(vec![], true, false);
    }

    fn escape(&mut self) -> MenuFrame {
        self.frame(
            vec![
                key(Key::Escape, true, false),
                key(Key::Escape, false, false),
            ],
            true,
            false,
        )
    }

    fn open(&mut self) {
        let frame = self.escape();
        assert!(frame.blocked && frame.changed && frame.action.is_none());
        assert!(self.menu.is_open());
        self.settle();
    }

    fn position(&self, label: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("missing {label}: {:?}", self.texts))
            .1
            .center()
    }

    fn click_at(&mut self, position: Pos2) -> MenuFrame {
        self.frame(
            vec![Event::PointerMoved(position), pointer(position, true)],
            true,
            false,
        );
        self.frame(vec![pointer(position, false)], true, false)
    }

    fn click(&mut self, label: &str) -> MenuFrame {
        let result = self.click_at(self.position(label));
        // Keep the action from the release frame while letting a new page settle.
        self.settle();
        result
    }

    fn settings_page(&mut self) {
        self.open();
        assert!(self.click("设置").action.is_none());
        assert!(self.menu.page == Some(Page::Settings));
    }
}

fn text_shapes(shape: &egui::epaint::Shape, texts: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::epaint::Shape::Text(text) => texts.push((
            text.galley.text().to_owned(),
            egui::Rect::from_min_size(text.pos, text.galley.size()),
        )),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                text_shapes(shape, texts);
            }
        }
        _ => {}
    }
}

fn key(key: Key, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    }
}

fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn escape_opens_before_gameplay_and_focus_recovery_requires_a_new_press() {
    let mut ui = Harness::new();
    let ignored = ui.frame(vec![key(Key::Escape, true, false)], false, false);
    assert!(!ignored.blocked && !ui.menu.is_open());
    assert!(!ui.frame(vec![], true, true).blocked);
    assert!(
        !ui.frame(vec![key(Key::Escape, true, true)], true, false)
            .blocked
    );
    ui.frame(vec![key(Key::Escape, false, false)], true, false);
    let opened = ui.frame(vec![key(Key::Escape, true, false)], true, false);
    assert!(opened.blocked && opened.changed && opened.action.is_none());
    assert!(ui.menu.is_open());
    let repeat = ui.frame(vec![key(Key::Escape, true, true)], true, false);
    assert!(repeat.blocked && !repeat.changed && repeat.action.is_none());
    ui.frame(vec![key(Key::Escape, false, false)], true, false);
    let resumed = ui.escape();
    assert!(resumed.blocked && resumed.changed);
    assert_eq!(resumed.action, Some(MenuAction::Resume));
    assert!(!ui.menu.is_open());
    assert!(!ui.frame(vec![], true, false).blocked);
}

#[test]
fn real_buttons_emit_one_action_and_failure_feedback_keeps_the_menu_open() {
    let mut ui = Harness::new();
    ui.menu
        .feedback(Some("读取设置失败，已使用默认设置".into()));
    ui.open();
    ui.position("读取设置失败，已使用默认设置");
    ui.menu.feedback(None);
    ui.settle();
    for (label, expected) in [
        ("保存进度", MenuAction::Save),
        ("继续存档", MenuAction::Continue),
        ("退出游戏", MenuAction::Exit),
    ] {
        let frame = ui.click(label);
        assert!(frame.blocked && !frame.changed && ui.menu.is_open());
        assert_eq!(frame.action, Some(expected));
        assert!(ui.frame(vec![], true, false).action.is_none());
    }
    ui.menu
        .feedback(Some("存档读取失败，当前运行保持不变".into()));
    ui.frame(vec![], true, false);
    ui.position("存档读取失败，当前运行保持不变");
    assert!(ui.menu.is_open());
    ui.menu.feedback(None);
    ui.settle();
    let returned = ui.click("返回游戏");
    assert!(returned.blocked && returned.changed);
    assert_eq!(returned.action, Some(MenuAction::Resume));
    assert!(!ui.menu.is_open());
    ui.open();
    let continuing = ui.click("继续存档");
    assert!(continuing.blocked && ui.menu.close());
    assert!(!ui.menu.close());
}

#[test]
fn pointer_release_after_focus_recovery_is_swallowed_before_controls_reenable() {
    let mut ui = Harness::new();
    ui.open();
    let exit = ui.position("退出游戏");
    ui.frame(
        vec![Event::PointerMoved(exit), pointer(exit, true)],
        true,
        false,
    );
    assert!(ui.frame(vec![], false, true).action.is_none());
    assert!(ui.frame(vec![], true, true).action.is_none());
    let released = ui.frame(vec![pointer(exit, false)], true, false);
    assert!(released.action.is_none());
    assert!(released.blocked && ui.menu.is_open());
    ui.settle();
    assert_eq!(ui.click("退出游戏").action, Some(MenuAction::Exit));
}

#[test]
fn cancelling_a_volume_drag_restores_its_start_and_swallows_the_held_pointer() {
    let mut ui = Harness::new();
    ui.settings_page();
    let position = ui.position("1.00");
    ui.frame(
        vec![Event::PointerMoved(position), pointer(position, true)],
        true,
        false,
    );
    let moved = position - egui::vec2(90., 0.);
    ui.frame(vec![Event::PointerMoved(moved)], true, false);
    assert!(ui.menu.draft.master_volume < 1.);
    assert!(ui.menu.edit.is_some());
    let cancelled = ui.escape();
    assert!(cancelled.blocked && !cancelled.changed && cancelled.action.is_none());
    assert!(ui.menu.page == Some(Page::Settings));
    assert_eq!(ui.menu.draft.master_volume, 1.);
    ui.frame(
        vec![Event::PointerMoved(moved - egui::vec2(20., 0.))],
        true,
        false,
    );
    assert_eq!(ui.menu.draft.master_volume, 1.);
    ui.frame(vec![pointer(moved, false)], true, false);
    ui.settle();
    assert!(ui.click("方向光阴影").action.is_none());
    assert!(!ui.menu.draft.shadows && ui.settings.shadows);
    let back = ui.escape();
    assert!(back.blocked && !back.changed && back.action.is_none());
    assert!(ui.menu.page == Some(Page::Main));
    assert_eq!(ui.menu.draft, ui.settings);
    assert_eq!(ui.escape().action, Some(MenuAction::Resume));
}

#[test]
fn settings_are_only_proposed_by_apply_and_failed_publication_can_be_retried() {
    let mut ui = Harness::new();
    ui.settings_page();
    ui.click("1.00");
    ui.frame(vec![Event::Text("0.25".into())], true, false);
    ui.frame(
        vec![key(Key::Enter, true, false), key(Key::Enter, false, false)],
        true,
        false,
    );
    ui.settle();
    assert_eq!(ui.menu.draft.master_volume, 0.25);
    ui.click("方向光阴影");
    let expected = Settings {
        master_volume: 0.25,
        shadows: false,
    };
    let applied = ui.click("应用");
    assert_eq!(applied.action, Some(MenuAction::Apply(expected)));
    assert_eq!(ui.settings, Settings::default());
    ui.menu.feedback(Some("保存设置失败".into()));
    ui.settle();
    ui.position("保存设置失败");
    assert_eq!(ui.menu.draft, expected);
    assert_eq!(ui.click("应用").action, Some(MenuAction::Apply(expected)));
    ui.settings = expected;
    ui.menu.feedback(None);
    ui.settle();
    assert!(ui.click("应用").action.is_none());
    assert!(ui.menu.is_open());
    ui.click("返回");
    assert!(ui.menu.page == Some(Page::Main));
    assert_eq!(ui.settings, expected);
}

#[test]
fn ime_composition_and_focused_text_do_not_turn_escape_into_menu_navigation() {
    let mut ui = Harness::new();
    ui.text_focus = true;
    ui.frame(vec![], true, false);
    let preedit = |text: &str| {
        Event::Ime(egui::ImeEvent::Preedit {
            text: text.into(),
            active_range_chars: None,
        })
    };
    let external = ui.frame(
        vec![preedit("输入"), key(Key::Escape, true, false)],
        true,
        false,
    );
    assert!(!external.blocked && !ui.menu.is_open());
    ui.text_focus = false;
    ui.frame(
        vec![preedit(""), key(Key::Escape, false, false)],
        true,
        false,
    );
    ui.settle();
    ui.settings_page();
    ui.click("1.00");
    ui.frame(vec![preedit("0.40")], true, false);
    let captured = ui.escape();
    assert!(captured.blocked && !captured.changed && captured.action.is_none());
    assert!(ui.menu.page == Some(Page::Settings));
    ui.frame(vec![preedit("")], true, false);
    assert!(ui.menu.is_open());
    assert!(ui.escape().action.is_none());
    assert!(ui.menu.is_open());
}
