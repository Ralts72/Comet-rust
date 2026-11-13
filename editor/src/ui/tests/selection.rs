use super::*;
use crate::selection::SelectMode;
use comet_core::EntityId;
use comet_render::{FrameFeedback, PickFeedback, ViewToken};

fn setup(editor: &mut Editor) -> Result<Vec<EntityId>> {
    editor.document = comet_editor_core::Document::open(comet_scene::SceneSnapshot {
        entities: ["实体甲", "实体乙", "实体丙"].map(EntityData::new).to_vec(),
        ..Default::default()
    })?;
    Ok(editor
        .document
        .shared_source()
        .base
        .entities
        .iter()
        .map(|entity| entity.id)
        .collect())
}
fn context(number: u64) -> FrameContext {
    FrameContext {
        gamepad: Default::default(),
        number,
        dt: 1. / 60.,
        active: true,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    }
}
fn label(output: &egui::FullOutput, text: &str, region: Rect) -> Pos2 {
    fn find(shape: &egui::epaint::Shape, text: &str, region: Rect) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(value) if value.galley.job.text == text => {
                let position = value.pos + value.galley.rect.center().to_vec2();
                region.contains(position).then_some(position)
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| find(shape, text, region))
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, text, region))
        .unwrap_or_else(|| panic!("missing label {text}"))
}
fn pointer(position: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
fn feedback(view: &ViewRequest, number: u64, entity: Option<EntityId>) -> FrameFeedback {
    let token = ViewToken {
        world: view.world,
        view: 1,
        target_generation: 1,
    };
    FrameFeedback {
        number,
        token,
        presented: true,
        backend: "test".into(),
        worker_thread: "test".into(),
        draw_count: 0,
        gpu_ms: 0.,
        pick: Some(PickFeedback {
            token,
            entities: entity.into_iter().collect(),
        }),
    }
}

#[test]
fn hierarchy_toggle_range_and_group_shortcuts_use_atomic_history() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = setup(&mut editor)?;
    let before = editor.document.shared_source();
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    let positions: Vec<_> = before
        .base
        .entities
        .iter()
        .map(|entity| {
            label(
                &output,
                &entity.name,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(220., 800.)),
            )
        })
        .collect();
    let mut number = 2;
    let mut click = |editor: &mut Editor, position, modifiers| {
        frame(
            &ctx,
            editor,
            &mut assets,
            number,
            vec![
                egui::Event::PointerMoved(position),
                pointer(position, true, modifiers),
            ],
        );
        number += 1;
        let view = frame(
            &ctx,
            editor,
            &mut assets,
            number,
            vec![pointer(position, false, modifiers)],
        );
        number += 1;
        view
    };
    click(&mut editor, positions[0], egui::Modifiers::NONE);
    click(&mut editor, positions[2], egui::Modifiers::COMMAND);
    assert_eq!(editor.selection.len(), 2);
    assert!(editor.selection.contains(ids[0]) && editor.selection.contains(ids[2]));
    click(&mut editor, positions[2], egui::Modifiers::COMMAND);
    assert_eq!(editor.selection.ids(), vec![ids[0]]);
    click(&mut editor, positions[2], egui::Modifiers::SHIFT);
    assert_eq!(editor.selection.len(), 3);
    let view = click(&mut editor, positions[1], egui::Modifiers::SHIFT);
    assert_eq!(view.selected.len(), 2);
    assert!(editor.selection.contains(ids[0]) && editor.selection.contains(ids[1]));
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        number,
        vec![key(egui::Key::D, egui::Modifiers::COMMAND)],
    );
    number += 1;
    assert_eq!(editor.document.source().base.entities.len(), 5);
    assert_eq!(editor.document.history_len(), 1);
    assert_eq!(editor.selection.len(), 2);
    assert!(editor.selection.ids().iter().all(|id| !ids.contains(id)));
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        number,
        vec![key(egui::Key::Delete, egui::Modifiers::NONE)],
    );
    number += 1;
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 2);
    assert!(editor.selection.is_empty());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        number,
        vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
    );
    assert_eq!(editor.document.source().base.entities.len(), 5);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn viewport_click_modes_survive_modifier_release_and_later_direct_selection_wins() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = setup(&mut editor)?;
    editor.selection.choose(Some(ids[0]), SelectMode::Replace);
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let position = view.viewport.right_top() + Vec2::new(-30., 70.);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![
            egui::Event::PointerMoved(position),
            pointer(position, true, egui::Modifiers::COMMAND),
        ],
    );
    let first = frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![pointer(position, false, egui::Modifiers::COMMAND)],
    );
    assert!(first.pick.is_some());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![pointer(position, true, egui::Modifiers::COMMAND)],
    );
    let second = frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        vec![pointer(position, false, egui::Modifiers::COMMAND)],
    );
    assert!(second.pick.is_some());
    // Feedback arrives after both clicks and after modifiers have been released.
    editor.feedback(&feedback(&first, 3, Some(ids[1])));
    editor.feedback(&feedback(&second, 5, Some(ids[2])));
    assert_eq!(editor.selection.len(), 3);
    editor.feedback(&feedback(&second, 5, Some(ids[2])));
    assert_eq!(editor.selection.len(), 3);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        6,
        vec![pointer(position, true, egui::Modifiers::NONE)],
    );
    let stale = frame(
        &ctx,
        &mut editor,
        &mut assets,
        7,
        vec![pointer(position, false, egui::Modifiers::NONE)],
    );
    editor.selection.choose(Some(ids[0]), SelectMode::Replace);
    editor.feedback(&feedback(&stale, 7, Some(ids[1])));
    assert_eq!(editor.selection.ids(), vec![ids[0]]);
    Ok(())
}

#[test]
fn deleting_text_in_the_inspector_never_deletes_selected_objects() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = setup(&mut editor)?;
    editor.selection.replace([ids[0]]);
    let name = editor.document.shared_source().base.entities[0]
        .name
        .clone();
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let (view, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    let position = label(
        &output,
        &name,
        Rect::from_min_max(Pos2::new(view.viewport.right(), 0.), Pos2::new(1280., 800.)),
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![
            egui::Event::PointerMoved(position),
            pointer(position, true, egui::Modifiers::NONE),
        ],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![pointer(position, false, egui::Modifiers::NONE)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![
            key(egui::Key::A, egui::Modifiers::COMMAND),
            key(egui::Key::Delete, egui::Modifiers::NONE),
        ],
    );
    assert_eq!(editor.document.source().base.entities.len(), 3);
    assert_eq!(editor.selection.ids(), vec![ids[0]]);
    editor.cancel()?;
    Ok(())
}

#[test]
fn duplicate_then_delete_in_one_frame_applies_to_the_new_selection() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = setup(&mut editor)?;
    editor.selection.replace([ids[0], ids[1]]);
    let before = editor.document.shared_source();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        vec![
            key(egui::Key::D, egui::Modifiers::COMMAND),
            key(egui::Key::Delete, egui::Modifiers::NONE),
        ],
    );
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(editor.document.history_len(), 2);
    assert!(editor.selection.is_empty());
    Ok(())
}

#[test]
fn later_select_all_in_the_same_frame_discards_an_earlier_viewport_click() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let ids = setup(&mut editor)?;
    editor.selection.replace([ids[0]]);
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let view = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let position = view.viewport.right_top() + Vec2::new(-30., 70.);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![
            egui::Event::PointerMoved(position),
            pointer(position, true, egui::Modifiers::NONE),
        ],
    );
    let all = frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![
            pointer(position, false, egui::Modifiers::NONE),
            key(egui::Key::A, egui::Modifiers::COMMAND),
        ],
    );
    assert_eq!(
        editor.selection.len(),
        3,
        "viewport focus must allow object shortcuts"
    );
    assert!(
        all.pick.is_none(),
        "the earlier click must not be submitted under a later selection generation"
    );
    editor.feedback(&feedback(&all, 3, Some(ids[1])));
    assert_eq!(editor.selection.len(), 3);
    // A subsequent shortcut remains available with the viewport focused.
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        vec![key(egui::Key::Delete, egui::Modifiers::NONE)],
    );
    assert!(editor.document.source().base.entities.is_empty());
    assert!(editor.selection.is_empty());
    Ok(())
}
