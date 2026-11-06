use super::*;

#[test]
fn inspector_name_edit_commits_once_and_escape_restores_the_whole_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.create_object(None, &mut assets)?;
    let id = editor.selection.active().unwrap();
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
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
    };
    render(&mut editor, vec![]);
    let (view, output) = render(&mut editor, vec![]);
    fn name_position(shape: &egui::epaint::Shape, x: f32) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text)
                if text.galley.job.text == "新实体" && text.pos.x > x =>
            {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| name_position(shape, x))
            }
            _ => None,
        }
    }
    let position = output
        .shapes
        .iter()
        .find_map(|shape| name_position(&shape.shape, view.viewport.max.x))
        .expect("inspector name field is visible");
    let button = |pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    render(
        &mut editor,
        vec![egui::Event::PointerMoved(position), button(true)],
    );
    render(&mut editor, vec![button(false)]);
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    render(
        &mut editor,
        vec![
            key(egui::Key::A, egui::Modifiers::COMMAND),
            egui::Event::Text("中文实体".into()),
        ],
    );
    assert_eq!(
        editor
            .document
            .source()
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .name,
        "中文实体"
    );
    assert!(editor.document.is_editing());
    assert_eq!(editor.document.history_len(), history);
    render(&mut editor, vec![egui::Event::Text("名称".into())]);
    render(
        &mut editor,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), history);

    // A second edit is committed by Enter, with the same stable object identity.
    render(
        &mut editor,
        vec![egui::Event::PointerMoved(position), button(true)],
    );
    render(&mut editor, vec![button(false)]);
    render(
        &mut editor,
        vec![
            key(egui::Key::A, egui::Modifiers::COMMAND),
            egui::Event::Text("重命名实体".into()),
        ],
    );
    render(
        &mut editor,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.history_len(), history + 1);
    assert_eq!(
        editor
            .document
            .source()
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .unwrap()
            .name,
        "重命名实体"
    );
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}
