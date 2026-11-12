mod documents;
mod gizmo;
mod hierarchy;
mod inspector;
mod project;
mod rename;
mod resources;
mod restart;
mod selection;
mod viewport;
mod workspace;

use comet_core::math::Vec3;
use comet_scene::EntityData;
use egui::{Pos2, Rect, Vec2};

use super::*;
use crate::selection::SelectMode;
use crate::test_support::fixture;
use comet_application::{Application, FrameContext, ViewRequest};
use std::fs;

#[test]
fn command_shortcuts_use_event_modifiers_even_when_released_before_the_frame() -> Result<()> {
    use comet_scene::{schema, schema::FieldValue};
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3([2., 3., 4.]),
        &mut assets,
    )?;
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    let events = |key, modifiers| {
        vec![
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            },
        ]
    };
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        0,
        events(egui::Key::S, egui::Modifiers::COMMAND),
    );
    assert!(!editor.document.is_dirty());
    assert!(!editor.document.is_editing());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        events(egui::Key::Z, egui::Modifiers::COMMAND),
    );
    assert!(editor.document.is_dirty());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        events(
            egui::Key::Z,
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        ),
    );
    assert!(!editor.document.is_dirty());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        3,
        events(egui::Key::N, egui::Modifiers::COMMAND),
    );
    assert!(editor.documents.is_blocking());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        4,
        events(egui::Key::Escape, egui::Modifiers::NONE),
    );
    assert!(!editor.documents.is_blocking());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        5,
        events(egui::Key::O, egui::Modifiers::COMMAND),
    );
    assert!(editor.documents.is_blocking());
    Ok(())
}

#[test]
fn cancelling_the_scene_browser_preserves_an_active_document_preview() -> Result<()> {
    use comet_scene::{schema, schema::FieldValue};
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3([2., 3., 4.]),
        &mut assets,
    )?;
    let preview = editor.document.shared_source();
    let saved = fs::read(&editor.source)?;
    editor
        .documents
        .open_browser(crate::documents::BrowserMode::NewScene);
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        1,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(!editor.documents.is_blocking());
    assert!(editor.document.is_editing());
    assert_eq!(editor.document.shared_source(), preview);
    assert_eq!(fs::read(&editor.source)?, saved);
    assert!(!editor.close_ready());
    Ok(())
}

#[test]
fn close_modal_blocks_shortcuts_and_freezes_play_until_cancelled() -> Result<()> {
    use comet_scene::{schema, schema::FieldValue};
    let (_temp, mut editor, mut assets) = fixture()?;
    let id = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3([2., 3., 4.]),
        &mut assets,
    )?;
    editor.start(&mut assets)?;
    let saved = fs::read(&editor.source)?;
    let authoring = editor.document.shared_source();
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    let steps = editor.play.as_ref().unwrap().steps();
    assert!(!editor.request_close());
    frame_with_output(
        &ctx,
        &mut editor,
        &mut assets,
        FrameContext {
            number: 1,
            dt: 1.,
            active: true,
            activity_changed: false,
            target_generation: 1,
            size: [1280, 800],
            smoke: false,
        },
        vec![egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    let (_, output) = frame_with_output(
        &ctx,
        &mut editor,
        &mut assets,
        FrameContext {
            number: 2,
            dt: 1.,
            active: true,
            activity_changed: false,
            target_generation: 1,
            size: [1280, 800],
            smoke: false,
        },
        vec![],
    );
    fn find_cancel(shape: &egui::epaint::Shape) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == "取消" => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(find_cancel),
            _ => None,
        }
    }
    let cancel = output
        .shapes
        .iter()
        .find_map(|s| find_cancel(&s.shape))
        .expect("close confirmation must show a cancel button");
    assert_eq!(editor.play.as_ref().unwrap().steps(), steps);
    assert_eq!(fs::read(&editor.source)?, saved);
    assert!(
        editor.error.is_none(),
        "background save shortcut must not run during confirmation"
    );
    assert_eq!(editor.document.shared_source(), authoring);
    for (number, pressed) in [(3, true), (4, false)] {
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            number,
            vec![
                egui::Event::PointerMoved(cancel),
                egui::Event::PointerButton {
                    pos: cancel,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert!(!editor.documents.pending);
    assert!(!editor.close_ready());
    assert!(editor.document.is_dirty());
    assert!(editor.play.is_some());
    assert_eq!(editor.document.shared_source(), authoring);
    Ok(())
}
fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    number: u64,
    events: Vec<egui::Event>,
) -> ViewRequest {
    frame_with_context(
        ctx,
        editor,
        assets,
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
}
fn frame_with_context(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    context: FrameContext,
    events: Vec<egui::Event>,
) -> ViewRequest {
    frame_with_output(ctx, editor, assets, context, events).0
}
fn frame_with_output(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    context: FrameContext,
    events: Vec<egui::Event>,
) -> (ViewRequest, egui::FullOutput) {
    let mut request = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))),
            events,
            ..Default::default()
        },
        |ui| {
            request = Some(editor.frame(ui, assets, &context).unwrap());
        },
    );
    output.textures_delta.clear();
    (request.unwrap(), output)
}
#[test]
fn physics_parameter_drag_keeps_its_preview_and_commits_one_history_item() -> Result<()> {
    use comet_scene::{BodyMotion, Character, ColliderShape, PhysicsBody};
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut source = editor.document.source();
    let mut entity = EntityData::new("角色");
    let id = entity.id;
    entity.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    entity.character = Some(Character::default());
    source.base.entities.push(entity);
    editor.document = comet_editor_core::Document::open_source(source)?;
    editor.selection.replace([id]);
    let before = editor.document.shared_source();
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
    let context = |number| FrameContext {
        number,
        dt: 1. / 60.,
        active: true,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    };
    frame_with_output(&ctx, &mut editor, &mut assets, context(0), vec![]);
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    fn field_position(shape: &egui::epaint::Shape) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text.starts_with("移动速度 ") => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(field_position),
            _ => None,
        }
    }
    let start = output
        .shapes
        .iter()
        .find_map(|s| field_position(&s.shape))
        .expect("movement speed control is visible");
    let end = start + Vec2::new(40., 0.);
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
    let changed = editor.document.shared_source();
    assert_ne!(
        changed, before,
        "dragging must change the physics configuration"
    );
    frame(&ctx, &mut editor, &mut assets, 4, vec![]);
    assert_eq!(editor.document.shared_source(), changed);
    frame(&ctx, &mut editor, &mut assets, 5, vec![button(end, false)]);
    assert_eq!(editor.document.history_len(), 1);
    assert!(!editor.document.is_editing());
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), before);
    assert!(!editor.document.is_dirty());
    Ok(())
}

#[test]
fn gizmo_drag_holds_its_value_and_commits_one_history_item() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let before = editor.snapshot(&mut assets)?;
    let selected = before
        .entities
        .iter()
        .find(|e| e.name == "Amber cube")
        .unwrap()
        .id;
    editor.selection.replace([selected]);
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

#[test]
fn play_frames_share_fixed_time_input_and_host_pause_policy() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let source = editor.document.source();
    editor.start(&mut assets)?;
    let world = editor.world();
    let ctx = egui::Context::default();
    ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
    let context = |dt, active| FrameContext {
        number: 0,
        dt,
        active,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    };
    frame_with_context(&ctx, &mut editor, &mut assets, context(0., true), vec![]);
    let key = |pressed| egui::Event::Key {
        key: egui::Key::Space,
        physical_key: Some(egui::Key::Space),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        context(comet_runtime::FIXED_DELTA / 2., true),
        vec![key(true), key(false)],
    );
    assert_eq!(editor.play.as_ref().unwrap().steps(), 0);
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        context(comet_runtime::FIXED_DELTA / 2., true),
        vec![],
    );
    let play = editor.play.as_ref().unwrap();
    assert_eq!(play.steps(), 1);
    assert_eq!(play.input().events.len(), 2);
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        context(0., true),
        vec![key(true), key(false)],
    );
    let mut restored = context(0., true);
    restored.activity_changed = true;
    frame_with_context(&ctx, &mut editor, &mut assets, restored, vec![]);
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        context(comet_runtime::FIXED_DELTA, true),
        vec![],
    );
    assert!(editor.play.as_ref().unwrap().input().events.is_empty());
    frame_with_context(&ctx, &mut editor, &mut assets, context(100., false), vec![]);
    assert!(editor.play.as_ref().unwrap().is_suspended());
    assert_eq!(editor.play.as_ref().unwrap().steps(), 2);
    frame_with_context(&ctx, &mut editor, &mut assets, context(0., true), vec![]);
    assert_eq!(editor.play.as_ref().unwrap().steps(), 2);
    editor.play.as_mut().unwrap().set_paused(true);
    editor.play.as_mut().unwrap().request_single_step()?;
    frame_with_context(&ctx, &mut editor, &mut assets, context(10., true), vec![]);
    let play = editor.play.as_ref().unwrap();
    assert_eq!(play.steps(), 3);
    assert_eq!(play.dropped_time(), 0.);
    assert_eq!(editor.world(), world);
    editor.stop();
    assert_eq!(editor.document.source(), source);
    assert!(!editor.document.is_dirty());
    editor.start(&mut assets)?;
    assert_ne!(editor.world(), world);
    assert_eq!(editor.play.as_ref().unwrap().steps(), 0);
    Ok(())
}

#[test]
fn editor_play_simulates_physics_and_restores_authoring_after_stop() -> Result<()> {
    use comet_scene::{BodyMotion, Character, ColliderShape, PhysicsBody};
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut source = editor.document.source();
    let mut floor = EntityData::new("floor");
    floor.transform.translation = [0., -0.5, 0.];
    floor.physics = Some(PhysicsBody {
        shape: ColliderShape::Box {
            half_extents: [20., 0.5, 20.],
        },
        ..Default::default()
    });
    let mut player = EntityData::new("player");
    let id = player.id;
    player.transform.translation = [0., 2., 3.];
    player.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    player.character = Some(Character::default());
    source.base.entities.extend([floor, player]);
    editor.document = comet_editor_core::Document::open_source(source.clone())?;
    editor.edit(
        id,
        comet_scene::schema::MOVE_SPEED,
        comet_scene::schema::FieldValue::Scalar(6.),
        &mut assets,
    )?;
    editor.commit()?;
    assert_eq!(editor.document.history_len(), 1);
    editor.document.undo()?;
    let before = editor.document.source();
    editor.start(&mut assets)?;
    let ctx = egui::Context::default();
    ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
    for n in 0..60 {
        frame(&ctx, &mut editor, &mut assets, n, vec![]);
    }
    assert!(
        editor
            .play
            .as_ref()
            .unwrap()
            .character(id)
            .unwrap()
            .grounded
    );
    let press = egui::Event::Key {
        key: egui::Key::W,
        physical_key: Some(egui::Key::W),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let camera = editor.camera;
    let position = editor
        .play
        .as_ref()
        .unwrap()
        .character(id)
        .unwrap()
        .position;
    for n in 60..75 {
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            n,
            if n == 60 { vec![press.clone()] } else { vec![] },
        );
    }
    let next = editor
        .play
        .as_ref()
        .unwrap()
        .character(id)
        .unwrap()
        .position;
    assert!(Vec3::from_array(next).distance(Vec3::from_array(position)) > 0.5);
    assert!(
        (editor.camera.target
            - camera.target
            - (Vec3::from_array(next) - Vec3::from_array(position)))
        .length()
            < 1e-5
    );
    editor.stop();
    assert_eq!(editor.document.source(), before);
    assert!(!editor.document.is_dirty());
    Ok(())
}
