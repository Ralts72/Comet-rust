use super::*;

fn text_center(output: &egui::FullOutput, label: &str) -> Pos2 {
    fn find(shape: &egui::epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
        .unwrap_or_else(|| panic!("missing label: {label}"))
}

#[test]
fn hierarchy_drag_reparents_and_root_drop_undoes_as_one_command() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.document = comet_editor_core::Document::open(Default::default())?;
    editor.create_object(None, &mut assets)?;
    let parent = editor.selection.active().unwrap();
    editor.create_object(None, &mut assets)?;
    let child = editor.selection.active().unwrap();
    let before = editor.document.shared_source();
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let mut number = 0;
    let mut render = |editor: &mut Editor, events| {
        number += 1;
        frame_with_output(
            &ctx,
            editor,
            &mut assets,
            FrameContext {
                number,
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
    };
    render(&mut editor, vec![]);
    let output = render(&mut editor, vec![]);
    let root_position = text_center(&output, "场景根节点");
    let parent_position = text_center(&output, "新对象");
    let child_position = text_center(&output, "新对象 2");
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut drag = |editor: &mut Editor, from, to| {
        render(
            editor,
            vec![egui::Event::PointerMoved(from), button(from, true)],
        );
        render(
            editor,
            vec![egui::Event::PointerMoved(from + Vec2::new(8., 0.))],
        );
        render(editor, vec![egui::Event::PointerMoved(to)]);
        render(editor, vec![button(to, false)]);
        render(editor, vec![])
    };
    let output = drag(&mut editor, child_position, parent_position);
    assert_eq!(
        editor
            .document
            .source()
            .base
            .entities
            .iter()
            .find(|e| e.id == child)
            .unwrap()
            .parent,
        Some(parent)
    );
    assert_eq!(editor.selection.active(), Some(child));
    assert_eq!(editor.document.history_len(), 3);
    let reparented = editor.document.shared_source();
    let child_position = text_center(&output, "新对象 2");
    drag(&mut editor, child_position, root_position);
    assert_eq!(
        editor
            .document
            .source()
            .base
            .entities
            .iter()
            .find(|e| e.id == child)
            .unwrap()
            .parent,
        None
    );
    assert_eq!(editor.document.history_len(), 4);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), reparented);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn dragging_a_selected_member_reparents_the_whole_group_in_one_history_step() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let a = EntityData::new("移动甲");
    let b = EntityData::new("移动乙");
    let parent = EntityData::new("目标父级");
    let ids = [a.id, b.id];
    let parent_id = parent.id;
    editor.document = comet_editor_core::Document::open(comet_scene::SceneSnapshot {
        entities: vec![a, b, parent],
        ..Default::default()
    })?;
    editor.selection.replace(ids);
    let before = editor.document.shared_source();
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let mut number = 0;
    let mut render = |editor: &mut Editor, events| {
        number += 1;
        frame_with_output(
            &ctx,
            editor,
            &mut assets,
            FrameContext {
                number,
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
    };
    render(&mut editor, vec![]);
    let output = render(&mut editor, vec![]);
    let from = text_center(&output, "移动甲");
    let to = text_center(&output, "目标父级");
    let root = text_center(&output, "场景根节点");
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut drag = |editor: &mut Editor, from, to| {
        render(
            editor,
            vec![egui::Event::PointerMoved(from), button(from, true)],
        );
        render(
            editor,
            vec![egui::Event::PointerMoved(from + Vec2::new(8., 0.))],
        );
        render(editor, vec![egui::Event::PointerMoved(to)]);
        render(editor, vec![button(to, false)]);
        render(editor, vec![])
    };
    let output = drag(&mut editor, from, to);
    assert_eq!(editor.selection.len(), 2);
    for id in ids {
        assert_eq!(
            editor
                .document
                .source()
                .base
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .unwrap()
                .parent,
            Some(parent_id)
        );
    }
    assert_eq!(editor.document.history_len(), 1);
    let grouped = editor.document.shared_source();
    drag(&mut editor, text_center(&output, "移动甲"), root);
    assert_eq!(editor.document.history_len(), 2);
    assert_eq!(editor.document.shared_source(), before);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), grouped);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}
