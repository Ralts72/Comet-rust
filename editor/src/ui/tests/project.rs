use super::*;
use comet_core::math::Vec3;

#[test]
fn cancelling_a_project_browser_preserves_the_active_translation_preview() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.create_object(None, &mut assets)?;
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, 0, vec![]);
    editor.begin_translation(&mut assets)?;
    editor.preview_translation(Vec3::X, &mut assets)?;
    let preview = editor.document.shared_source();
    let history = editor.document.history_len();
    let current = editor.project_root().to_owned();
    editor.documents.open_project_browser(true, &current);
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
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!editor.documents.is_blocking());
    assert_eq!(editor.document.shared_source(), preview);
    assert!(editor.document.is_editing());
    assert!(editor.drag_origin.is_some());
    assert_eq!(editor.document.history_len(), history);
    editor.cancel_transform()?;
    Ok(())
}

#[test]
fn project_shortcut_freezes_play_and_escape_cancels_a_focused_path_field() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    editor.start(&mut assets)?;
    let world = editor.world();
    let authoring = editor.document.shared_source();
    let current = editor.project_root().display().to_string();
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        0,
        vec![egui::Event::Key {
            key: egui::Key::O,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        }],
    );
    assert!(editor.documents.is_blocking());
    let steps = editor.play.as_ref().unwrap().steps();
    let context = |number| FrameContext {
        gamepad: Default::default(),
        number,
        dt: 1.,
        active: true,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    };
    let (_, output) = frame_with_output(&ctx, &mut editor, &mut assets, context(1), vec![]);
    fn field(shape: &egui::epaint::Shape, value: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == value => {
                Some(text.pos + egui::vec2(12., text.galley.size().y * 0.5))
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| field(shape, value)),
            _ => None,
        }
    }
    let position = output
        .shapes
        .iter()
        .find_map(|shape| field(&shape.shape, &current))
        .expect("project directory text is visible");
    let button = |pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        context(2),
        vec![egui::Event::PointerMoved(position), button(true)],
    );
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        context(3),
        vec![button(false)],
    );
    assert!(ctx.text_edit_focused());
    frame_with_context(
        &ctx,
        &mut editor,
        &mut assets,
        context(4),
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!editor.documents.is_blocking());
    assert_eq!(editor.world(), world);
    assert_eq!(editor.play.as_ref().unwrap().steps(), steps);
    assert_eq!(editor.document.shared_source(), authoring);
    assert!(!editor.project_load.is_running());
    Ok(())
}

#[test]
fn a_recent_project_menu_uses_the_existing_unsaved_confirmation_and_cancel_keeps_history()
-> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let recent_root = temp.path().join("recent-game");
    comet_asset_pipeline::create_project(&recent_root)?;
    let preferences = temp.path().join("preferences/recent-projects.json");
    editor.recent_projects = crate::project::RecentProjects::load(preferences.clone())?;
    editor.recent_projects.record(
        &comet_core::ProjectPaths::open(&recent_root)?,
        &recent_root.canonicalize()?.join("assets/main.scene"),
    )?;
    editor.create_object(None, &mut assets)?;
    let original = editor.document.shared_source();
    let world = editor.world();
    let original_preferences = fs::read(&preferences)?;
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    fn text_position(shape: &egui::epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| text_position(shape, label))
            }
            _ => None,
        }
    }
    let mut number = 0;
    let context = |number| FrameContext {
        gamepad: Default::default(),
        number,
        dt: 1. / 60.,
        active: true,
        activity_changed: false,
        target_generation: 1,
        size: [1280, 800],
        smoke: false,
    };
    for label in ["文件", "最近项目", "recent-game", "取消"] {
        let (_, output) =
            frame_with_output(&ctx, &mut editor, &mut assets, context(number), vec![]);
        number += 1;
        let position = output
            .shapes
            .iter()
            .find_map(|shape| text_position(&shape.shape, label))
            .unwrap_or_else(|| panic!("visible menu or confirmation item: {label}"));
        for pressed in [true, false] {
            frame_with_context(
                &ctx,
                &mut editor,
                &mut assets,
                context(number),
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            number += 1;
        }
        if label == "recent-game" {
            assert!(editor.documents.pending);
            assert!(!editor.project_load.is_running());
            assert_eq!(editor.world(), world);
        }
    }
    assert!(!editor.documents.is_blocking());
    assert!(editor.document.is_dirty());
    assert_eq!(editor.document.shared_source(), original);
    assert_eq!(editor.world(), world);
    assert_eq!(fs::read(&preferences)?, original_preferences);
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_project_browser_draws_without_changing_authoring_or_user_preferences() -> Result<()> {
    let (_temp, mut editor, assets) = fixture()?;
    let source = editor.source.clone();
    let bytes = fs::read(&source)?;
    let current = editor.project_root().to_owned();
    editor.documents.open_project_browser(false, &current);
    assert!(!editor.recent_projects.is_enabled());
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    let mut options = comet_application::HostOptions::new("Comet · 项目浏览");
    options.headless = true;
    options.smoke = true;
    options.report = Some(out.join("editor-project-browser.json"));
    options.capture = Some(out.join("editor-project-browser.png"));
    comet_application::run(options, assets, editor)?;
    assert_eq!(fs::read(source)?, bytes);
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("editor-project-browser.json"))?)?;
    assert_eq!(report["application"]["dirty"], false);
    assert_eq!(report["application"]["history"], 0);
    assert_eq!(report["application"]["play_active"], false);
    assert!(report["application"]["error"].is_null());
    assert_eq!(report["audio"]["state"], "idle");
    assert_eq!(report["submitted_frames"], 8);
    Ok(())
}
