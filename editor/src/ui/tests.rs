use egui::Pos2;
use glam::Vec3;

use super::*;
use comet_application::{Application, FrameContext, ViewRequest};
use std::{fs, path::Path};
fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    number: u64,
    events: Vec<egui::Event>,
) -> ViewRequest {
    let mut request = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))),
            events,
            ..Default::default()
        },
        |ui| {
            request = Some(
                editor
                    .frame(
                        ui,
                        assets,
                        &FrameContext {
                            number,
                            dt: 1. / 60.,
                            target_generation: 1,
                            size: [1280, 800],
                            smoke: false,
                        },
                    )
                    .unwrap(),
            );
        },
    );
    output.textures_delta.clear();
    request.unwrap()
}
#[test]
fn gizmo_drag_holds_its_value_and_commits_one_history_item() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("assets"))?;
    for file in
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/gltf/assets"))?
    {
        let file = file?;
        if file.file_name() != ".assetc.lock" {
            fs::copy(file.path(), project.join("assets").join(file.file_name()))?;
        }
    }
    let (mut editor, mut assets) =
        crate::session::open(project, temp.path().join("products"), "main.scene".into())?;
    let before = editor.snapshot(&mut assets)?;
    let selected = before
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    editor.selected = Some(selected);
    let ctx = egui::Context::default();
    ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
    let first = frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let stable = frame(&ctx, &mut editor, &mut assets, 1, vec![]);
    let world = comet_application::world_transforms(&first.snapshot)?[&selected];
    let point = world.transform_point3(Vec3::ZERO) + Vec3::X * 0.55;
    let ndc = stable
        .camera
        .view_projection(stable.viewport.aspect_ratio())
        .project_point3(point);
    let start = Pos2::new(
        stable.viewport.center().x + ndc.x * stable.viewport.width() * 0.5,
        stable.viewport.center().y - ndc.y * stable.viewport.height() * 0.5,
    );
    let end = start + Vec2::new(45., 8.);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        vec![egui::Event::PointerMoved(end)],
    );
    let moved = editor.snapshot(&mut assets)?;
    assert_ne!(moved, before, "gizmo did not change the selected object");
    frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    assert_eq!(
        editor.snapshot(&mut assets)?,
        moved,
        "holding still reset the drag"
    );
    frame(&ctx, &mut editor, &mut assets, 5, vec![button(end, false)]);
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    assert_eq!(editor.snapshot(&mut assets)?, before);
    Ok(())
}
