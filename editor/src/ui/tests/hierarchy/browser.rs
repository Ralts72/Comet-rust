use super::*;
use comet_core::EntityId;
use std::sync::Arc;

struct Case {
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    number: u64,
}
impl Case {
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.number += 1;
        frame_with_output(
            &self.ctx,
            &mut self.editor,
            &mut self.assets,
            FrameContext {
                gamepad: Default::default(),
                number: self.number,
                dt: 1. / 60.,
                active: true,
                activity_changed: false,
                target_generation: 1,
                size: [1280, 800],
                smoke: false,
            },
            events,
        )
        .1
    }
    fn click(&mut self, pos: Pos2, button: egui::PointerButton, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                },
            ]);
        }
    }
    fn press(&mut self, label: &str) {
        let out = self.frame(vec![]);
        let pos = label_rect(&out, label)
            .unwrap_or_else(|| panic!("missing hierarchy label: {label}"))
            .center();
        self.click(pos, egui::PointerButton::Primary, egui::Modifiers::NONE);
    }
}
fn labels(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn collect(shape: &egui::epaint::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::epaint::Shape::Text(text) if text.pos.x < 220. => {
                out.push((
                    text.galley.job.text.clone(),
                    Rect::from_min_size(text.pos, text.galley.size()),
                ));
            }
            egui::epaint::Shape::Vec(shapes) => {
                for s in shapes {
                    collect(s, out);
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, &mut result);
    }
    result
}
fn label_rect(output: &egui::FullOutput, label: &str) -> Option<Rect> {
    labels(output)
        .into_iter()
        .find(|(text, _)| text == label)
        .map(|(_, rect)| rect)
}
fn entity(n: u32, name: &str, parent: Option<EntityId>) -> EntityData {
    let mut entity = EntityData::new(name);
    entity.id = format!("a0000000-0000-4000-8000-{n:012x}").parse().unwrap();
    entity.parent = parent;
    entity
}

#[test]
fn hierarchy_browsing_virtualizes_reveals_filters_ranges_and_preserves_authoring() -> Result<()> {
    let (_temp, mut editor, assets) = fixture()?;
    let saved = fs::read(&editor.source)?;
    editor.document = comet_editor_core::Document::open(comet_scene::SceneSnapshot {
        entities: (1..=2000)
            .map(|n| entity(n, &format!("节点 {n:04}"), None))
            .collect(),
        ..Default::default()
    })?;
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
    let mut c = Case {
        editor,
        assets,
        ctx,
        number: 0,
    };
    c.frame(vec![]);
    let output = c.frame(vec![]);
    let painted = labels(&output)
        .iter()
        .filter(|(text, _)| text.starts_with("节点 "))
        .count();
    assert!(
        (1..40).contains(&painted),
        "virtualization drew {painted} rows"
    );
    let last = c
        .editor
        .document
        .shared_source()
        .base
        .entities
        .last()
        .unwrap()
        .clone();
    c.editor.selection.replace([last.id]);
    c.frame(vec![]);
    assert!(label_rect(&c.frame(vec![]), &last.name).is_some());
    assert_eq!(c.editor.document.history_len(), 0);

    let root = entity(1, "作者根", None);
    let root_id = root.id;
    let a = entity(2, "匹配甲", Some(root_id));
    let a_id = a.id;
    let grand = entity(3, "孙节点", Some(a_id));
    let grand_id = grand.id;
    let b = entity(4, "匹配乙", Some(root_id));
    let b_id = b.id;
    c.editor.document = comet_editor_core::Document::open(comet_scene::SceneSnapshot {
        entities: vec![root, a, grand, b, entity(5, "外部根", None)],
        ..Default::default()
    })?;
    c.editor.selection.replace([grand_id]);
    c.frame(vec![]);
    c.frame(vec![]);
    let before = c.editor.document.shared_source();
    let revision = c.editor.document.revision();
    let out = c.frame(vec![]);
    let root_label = label_rect(&out, "作者根").unwrap();
    c.click(
        Pos2::new(root_label.left() - 14., root_label.center().y),
        egui::PointerButton::Primary,
        egui::Modifiers::NONE,
    );
    let folded = c.frame(vec![]);
    assert!(label_rect(&folded, "孙节点").is_none());
    assert_eq!(c.editor.selection.ids(), [grand_id]);
    assert!(
        labels(&folded)
            .iter()
            .any(|(text, _)| text.contains("隐藏 1"))
    );
    c.press("搜索实体...");
    c.frame(vec![egui::Event::Text("匹配".into())]);
    let filtered = c.frame(vec![]);
    assert!(label_rect(&filtered, "作者根").is_some());
    assert!(label_rect(&filtered, "匹配甲").is_some());
    assert!(label_rect(&filtered, "匹配乙").is_some());
    assert!(label_rect(&filtered, "孙节点").is_none());
    assert!(label_rect(&filtered, "外部根").is_none());
    for pressed in [true, false] {
        c.frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    let escaped = c.frame(vec![]);
    assert!(label_rect(&escaped, "搜索实体...").is_some());
    assert!(label_rect(&escaped, "匹配甲").is_none());
    assert!(!c.ctx.egui_wants_keyboard_input());
    c.press("搜索实体...");
    c.frame(vec![egui::Event::Text("匹配".into())]);
    let filtered = c.frame(vec![]);
    let a_pos = label_rect(&filtered, "匹配甲").unwrap().center();
    c.click(a_pos, egui::PointerButton::Primary, egui::Modifiers::NONE);
    let out = c.frame(vec![]);
    let b_pos = label_rect(&out, "匹配乙").unwrap().center();
    c.click(b_pos, egui::PointerButton::Primary, egui::Modifiers::SHIFT);
    assert_eq!(c.editor.selection.ids(), [a_id, b_id]);
    c.press("清除");
    let cleared = c.frame(vec![]);
    assert!(
        label_rect(&cleared, "匹配甲").is_none(),
        "filter must not overwrite folded state"
    );
    c.press("定位选中");
    let revealed = c.frame(vec![]);
    assert!(label_rect(&revealed, "匹配乙").is_some());
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.revision(), revision);
    assert_eq!(c.editor.document.history_len(), 0);
    assert!(!c.editor.document.is_dirty());

    c.press("全部折叠");
    c.frame(vec![]);
    let out = c.frame(vec![]);
    let root_pos = label_rect(&out, "作者根").unwrap().center();
    c.click(
        root_pos,
        egui::PointerButton::Secondary,
        egui::Modifiers::NONE,
    );
    let out = c.frame(vec![]);
    c.click(
        text_center(&out, "创建子实体"),
        egui::PointerButton::Primary,
        egui::Modifiers::NONE,
    );
    c.frame(vec![]);
    let created = c.editor.selection.active().unwrap();
    let data = c.editor.document.shared_source();
    let child = data.base.entities.iter().find(|e| e.id == created).unwrap();
    assert_eq!(child.parent, Some(root_id));
    assert!(
        label_rect(&c.frame(vec![]), &child.name).is_some(),
        "new child must reveal its folded parent"
    );
    assert_eq!(c.editor.document.history_len(), 1);
    c.editor.document.undo()?;
    c.frame(vec![]);
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    assert_eq!(fs::read(&c.editor.source)?, saved);
    Ok(())
}
