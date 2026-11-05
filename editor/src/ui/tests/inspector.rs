use crate::session::Editor;
use crate::test_support::fixture;
use anyhow::{Context, Result};
use comet_asset::AssetStore;
use egui::{Event, PointerButton, Pos2, Vec2};

fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    id: comet_core::EntityId,
    events: Vec<Event>,
) -> Result<egui::FullOutput> {
    let entity = editor
        .snapshot(assets)?
        .entities
        .into_iter()
        .find(|entity| entity.id == id)
        .context("inspector entity")?;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(640., 1200.),
            )),
            events,
            ..Default::default()
        },
        |ui| crate::ui::inspector::draw(editor, ui, assets, &entity),
    );
    output.textures_delta.clear();
    Ok(output)
}

#[test]
fn dragging_a_prefab_value_back_to_its_start_preserves_inheritance_and_history() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .context("prefab fixture")?
        .id;
    let before = editor.document.shared_source();
    let saved = std::fs::read(&editor.source)?;
    let state = editor.document.state_id();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, id, vec![])?;
    let output = frame(&ctx, &mut editor, &mut assets, id, vec![])?;
    fn position(shape: &egui::epaint::Shape) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text.starts_with("X ") => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(position),
            _ => None,
        }
    }
    let start = output
        .shapes
        .iter()
        .find_map(|shape| position(&shape.shape))
        .context("translation X control")?;
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![Event::PointerMoved(start), button(start, true)],
    )?;
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![Event::PointerMoved(start + Vec2::new(40., 0.))],
    )?;
    assert!(editor.document.is_editing());
    assert!(editor.document.is_dirty());
    assert_ne!(editor.document.shared_source(), before);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![Event::PointerMoved(start)],
    )?;
    assert_eq!(editor.document.shared_source(), before);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        id,
        vec![button(start, false)],
    )?;
    assert!(!editor.document.is_editing());
    assert!(!editor.document.is_dirty());
    assert_eq!(editor.document.state_id(), state);
    assert_eq!(editor.document.history_len(), 0);
    assert_eq!(editor.document.shared_source(), before);
    assert_eq!(std::fs::read(&editor.source)?, saved);
    Ok(())
}
