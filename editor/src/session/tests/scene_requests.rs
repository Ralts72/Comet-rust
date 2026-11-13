use super::*;
use comet_core::math::Vec3;
use comet_scene::{BodyMotion, ColliderShape, Script, ScriptValue};
use std::sync::Arc;

pub(super) fn write_asset(editor: &Editor, name: &str, bytes: &[u8]) -> Result<AssetId> {
    let path = editor.paths.assets().join(name);
    let meta = comet_asset_pipeline::SourceMeta {
        version: 1,
        id: AssetId::new(),
    };
    std::fs::write(&path, bytes)?;
    std::fs::write(
        comet_asset_pipeline::SourceMeta::path(&path),
        serde_json::to_vec(&meta)?,
    )?;
    Ok(meta.id)
}

pub(super) fn publish(editor: &Editor, assets: &mut AssetStore) -> Result<()> {
    comet_asset_pipeline::prepare(editor.paths.root(), &editor.products)?;
    assets.refresh()?;
    Ok(())
}

pub(super) fn bind_author(
    editor: &mut Editor,
    assets: &mut AssetStore,
    script: Script,
) -> Result<EntityId> {
    let owner = EntityData::new("author script");
    let id = owner.id;
    editor.document.insert_entity(owner, assets)?;
    editor.edit_script(id, Some(script), assets)?;
    editor.commit()?;
    Ok(id)
}

pub(super) fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    number: u64,
    active: bool,
    events: Vec<egui::Event>,
) -> Result<ViewRequest> {
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let mut request = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            request = Some(editor.frame(
                ui,
                assets,
                &FrameContext {
                    gamepad: Default::default(),
                    number,
                    dt: comet_runtime::FIXED_DELTA,
                    active,
                    activity_changed: false,
                    target_generation: 1,
                    size: [1280, 800],
                    smoke: false,
                },
            ));
        },
    );
    output.textures_delta.clear();
    let request = request.unwrap()?;
    // Exercise the host's extraction boundary without a native GPU or error recovery hiding a swap.
    comet_application::extract(&request.snapshot, assets, &request.selected)?;
    assert!(editor.error.is_none(), "{:?}", editor.error);
    Ok(request)
}

pub(super) fn scene_with_script(name: &str, script: AssetId) -> ContentSource {
    let mut owner = EntityData::new(name);
    owner.script = Some(Script::new(script));
    ContentSource {
        base: SceneSnapshot {
            entities: vec![owner],
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn requests_switch_only_the_play_world_and_restart_uses_the_current_scene() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let script_b = write_asset(
        &editor,
        "b.rhai",
        b"fn on_start(ctx) { ctx.set_hud(\"B task\"); ctx.log(\"B start\"); } fn on_stop(ctx) { ctx.log(\"B stop\"); }",
    )?;
    let mut scene_b = scene_with_script("B", script_b);
    let b = scene_b.base.entities[0].id;
    let mut player = EntityData::new("B player");
    player.transform.translation = [9., 3., 2.];
    player.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    player.character = Some(Character::default());
    scene_b.base.entities.push(player);
    let target = write_asset(&editor, "b.scene", &scene_b.to_json()?)?;
    let script_a = write_asset(
        &editor,
        "a.rhai",
        format!(
            "fn on_start(ctx) {{ ctx.set_hud(\"A task\"); ctx.request_scene(\"{target}\"); }} fn on_stop(ctx) {{ ctx.log(\"A stop\"); }}"
        )
        .as_bytes(),
    )?;
    publish(&editor, &mut assets)?;
    let a = bind_author(&mut editor, &mut assets, Script::new(script_a))?;
    let source = editor.document.shared_source();
    let history = editor.document.history_len();
    let bytes = std::fs::read(&editor.source)?;
    let revision = assets.revision().to_owned();
    editor.start(&mut assets)?;
    let world_a = editor.world();
    let hud_a = editor.play.as_ref().unwrap().hud_frame().unwrap();
    assert_eq!(hud_a.world, world_a);
    assert_eq!(hud_a.labels[0].text.as_ref(), "A task");
    editor
        .selection
        .choose(Some(a), crate::selection::SelectMode::Replace);
    editor.begin_scale(&mut assets)?;
    editor.play.as_mut().unwrap().set_paused(true);
    let ctx = egui::Context::default();
    // Loading must use accepted products, even if the authoring file changes after preparation.
    std::fs::write(
        temp.path().join("project/assets/b.scene"),
        b"unprepared source",
    )?;
    let view = frame(&ctx, &mut editor, &mut assets, 0, true, vec![])?;
    let world_b = editor.world();
    assert_ne!(world_b, world_a);
    assert_eq!(view.world, world_b);
    let hud = view.hud.as_ref().unwrap();
    assert_eq!(hud.world, world_b);
    assert_eq!(hud.labels[0].owner, b);
    assert_eq!(hud.labels[0].text.as_ref(), "B task");
    assert_eq!(editor.report()["hud"]["world"], serde_json::json!(world_b));
    assert!(view.snapshot.entities.iter().any(|entity| entity.id == b));
    assert!(!view.snapshot.entities.iter().any(|entity| entity.id == a));
    assert!(editor.selection.is_empty() && editor.drag_origin.is_none());
    let play = editor.play.as_ref().unwrap();
    assert!(play.is_paused() && !play.is_suspended());
    assert_eq!(play.steps(), 0);
    assert_eq!(editor.camera.target, Vec3::new(9., 3., 2.));
    assert!(editor.console.entries().iter().any(|entry| {
        entry.world == world_a && entry.phase == "on_stop" && entry.message == "A stop"
    }));
    assert!(editor.console.entries().iter().any(|entry| {
        entry.world == world_b && entry.phase == "on_start" && entry.message == "B start"
    }));
    assert_eq!(assets.revision(), revision);
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    let baseline = view.snapshot;
    editor.play.as_mut().unwrap().set_field(
        &[b],
        schema::TRANSLATION,
        FieldValue::Vec3([5., 0., 0.]),
    )?;
    frame(&ctx, &mut editor, &mut assets, 1, true, vec![])?;
    let restarted = frame(
        &ctx,
        &mut editor,
        &mut assets,
        2,
        true,
        vec![egui::Event::Key {
            key: egui::Key::R,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )?;
    assert_ne!(restarted.world, world_b);
    assert_eq!(restarted.snapshot, baseline);
    assert_eq!(restarted.hud.as_ref().unwrap().world, restarted.world);
    assert_eq!(
        restarted.hud.as_ref().unwrap().labels[0].text.as_ref(),
        "B task"
    );
    assert!(editor.play.as_ref().unwrap().is_paused());
    assert_eq!(editor.play.as_ref().unwrap().steps(), 0);
    editor.stop();
    assert!(
        frame(&ctx, &mut editor, &mut assets, 3, true, vec![])?
            .hud
            .is_none()
    );
    assert!(editor.report()["hud"].is_null());
    assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), history);
    assert!(editor.document.is_dirty());
    assert_eq!(std::fs::read(&editor.source)?, bytes);
    Ok(())
}

#[test]
fn rejected_requests_keep_the_old_world_and_the_requesting_binding_running() -> Result<()> {
    for failure in ["missing", "wrong kind", "script compile", "on_start draw"] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let target = match failure {
            "missing" => AssetId::new(),
            "wrong kind" => write_asset(&editor, "wrong.rhai", b"fn on_start(ctx) {}")?,
            _ => {
                let text = if failure == "script compile" {
                    "fn on_start("
                } else {
                    "fn on_start(ctx) { ctx.set_scale(ctx.entity(), 1e-40, 1e-40, 1e-40); }"
                };
                let script = write_asset(&editor, "bad.rhai", text.as_bytes())?;
                let mut content = scene_with_script("bad candidate", script);
                if failure == "on_start draw" {
                    let snapshot = editor.snapshot(&mut assets)?;
                    content.base.entities[0].model = snapshot
                        .entities
                        .iter()
                        .find_map(|entity| entity.model.clone());
                    assert!(content.base.entities[0].model.is_some());
                }
                write_asset(&editor, "bad.scene", &content.to_json()?)?
            }
        };
        let source = write_asset(
            &editor,
            "request.rhai",
            b"fn on_fixed_update(ctx, dt) { ctx.translate_local(ctx.entity(), 1, 0, 0); ctx.request_scene(ctx.param(\"next\")); }",
        )?;
        publish(&editor, &mut assets)?;
        let mut script = Script::new(source);
        script
            .parameters
            .insert("next".into(), ScriptValue::Text(target.to_string()));
        let owner = bind_author(&mut editor, &mut assets, script)?;
        let author = editor.document.shared_source();
        let history = editor.document.history_len();
        let bytes = std::fs::read(&editor.source)?;
        editor.start(&mut assets)?;
        let world = editor.world();
        let ctx = egui::Context::default();
        for number in 0..3 {
            let view = frame(&ctx, &mut editor, &mut assets, number, true, vec![])?;
            assert_eq!(view.world, world, "{failure}");
            assert_eq!(
                view.snapshot
                    .entities
                    .iter()
                    .find(|entity| entity.id == owner)
                    .unwrap()
                    .transform
                    .translation[0],
                (number + 1) as f32,
                "{failure}"
            );
        }
        let failures: Vec<_> = editor
            .console
            .entries()
            .iter()
            .filter(|entry| entry.failed)
            .collect();
        assert_eq!(failures.len(), 3, "{failure}: {failures:?}");
        assert!(failures.iter().all(|entry| {
            entry.world == world
                && entry.entity == owner
                && entry.asset == source
                && entry.phase == "on_fixed_update"
                && entry.message.contains(&target.to_string())
        }));
        editor.stop();
        assert!(Arc::ptr_eq(&author, &editor.document.shared_source()));
        assert_eq!(editor.document.history_len(), history);
        assert_eq!(std::fs::read(&editor.source)?, bytes);
    }
    Ok(())
}

#[test]
fn a_new_scenes_initialization_request_waits_for_the_next_frame() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let mut scene_c = ContentSource::default();
    scene_c.base.entities.push(EntityData::new("C"));
    let c = scene_c.base.entities[0].id;
    let target_c = write_asset(&editor, "c.scene", &scene_c.to_json()?)?;
    let script_b = write_asset(
        &editor,
        "b.rhai",
        format!("fn on_start(ctx) {{ ctx.request_scene(\"{target_c}\"); }}").as_bytes(),
    )?;
    let scene_b = scene_with_script("B", script_b);
    let b = scene_b.base.entities[0].id;
    let target_b = write_asset(&editor, "b.scene", &scene_b.to_json()?)?;
    let script_a = write_asset(
        &editor,
        "a.rhai",
        format!("fn on_start(ctx) {{ ctx.request_scene(\"{target_b}\"); }}").as_bytes(),
    )?;
    publish(&editor, &mut assets)?;
    bind_author(&mut editor, &mut assets, Script::new(script_a))?;
    editor.start(&mut assets)?;
    let ctx = egui::Context::default();
    let first = frame(&ctx, &mut editor, &mut assets, 0, true, vec![])?;
    assert_eq!(first.snapshot.entities.len(), 1);
    assert_eq!(first.snapshot.entities[0].id, b);
    let second = frame(&ctx, &mut editor, &mut assets, 1, true, vec![])?;
    assert_ne!(first.world, second.world);
    assert_eq!(second.snapshot.entities.len(), 1);
    assert_eq!(second.snapshot.entities[0].id, c);
    assert_eq!(editor.play.as_ref().unwrap().steps(), 0);
    let third = frame(&ctx, &mut editor, &mut assets, 2, true, vec![])?;
    assert_eq!(third.world, second.world);
    Ok(())
}

#[test]
fn pending_requests_survive_inactive_hosts_and_existing_or_new_document_modals() -> Result<()> {
    for blocker in ["inactive", "confirmation", "new browser"] {
        let (_temp, mut editor, mut assets) = fixture()?;
        let script_b = write_asset(
            &editor,
            "b.rhai",
            b"fn on_start(ctx) { ctx.log(\"B start\"); }",
        )?;
        let scene_b = scene_with_script("B", script_b);
        let b = scene_b.base.entities[0].id;
        let target = write_asset(&editor, "b.scene", &scene_b.to_json()?)?;
        let script_a = write_asset(
            &editor,
            "a.rhai",
            format!("fn on_start(ctx) {{ ctx.request_scene(\"{target}\"); }}").as_bytes(),
        )?;
        publish(&editor, &mut assets)?;
        bind_author(&mut editor, &mut assets, Script::new(script_a))?;
        let source = editor.document.shared_source();
        let history = editor.document.history_len();
        editor.start(&mut assets)?;
        editor.play.as_mut().unwrap().set_paused(true);
        let world = editor.world();
        if blocker == "confirmation" {
            editor.request_document_action(crate::documents::DocumentAction::Exit);
        }
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let events = if blocker == "new browser" {
            vec![key(egui::Key::O, egui::Modifiers::COMMAND)]
        } else {
            vec![]
        };
        let ctx = egui::Context::default();
        let first = frame(
            &ctx,
            &mut editor,
            &mut assets,
            0,
            blocker != "inactive",
            events,
        )?;
        assert_eq!(first.world, world, "{blocker}");
        let blocked = frame(
            &ctx,
            &mut editor,
            &mut assets,
            1,
            blocker != "inactive",
            vec![],
        )?;
        assert_eq!(blocked.world, world, "{blocker}");
        assert!(editor.play.as_ref().unwrap().is_suspended());
        if blocker != "inactive" {
            assert!(editor.documents.is_blocking());
            frame(
                &ctx,
                &mut editor,
                &mut assets,
                2,
                true,
                vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            )?;
            assert!(!editor.documents.is_blocking());
        }
        let accepted = frame(&ctx, &mut editor, &mut assets, 3, true, vec![])?;
        assert_ne!(accepted.world, world, "{blocker}");
        assert_eq!(accepted.snapshot.entities.len(), 1);
        assert_eq!(accepted.snapshot.entities[0].id, b);
        assert!(editor.play.as_ref().unwrap().is_paused());
        assert_eq!(editor.play.as_ref().unwrap().steps(), 0);
        let next = frame(&ctx, &mut editor, &mut assets, 4, true, vec![])?;
        assert_eq!(next.world, accepted.world);
        assert_eq!(
            editor
                .console
                .entries()
                .iter()
                .filter(|entry| entry.message == "B start")
                .count(),
            1
        );
        assert!(Arc::ptr_eq(&source, &editor.document.shared_source()));
        assert_eq!(editor.document.history_len(), history);
    }
    Ok(())
}

#[test]
fn initial_play_rejects_a_scene_made_undrawable_by_on_start() -> Result<()> {
    let (_temp, mut editor, mut assets) = fixture()?;
    let script = write_asset(
        &editor,
        "bad.rhai",
        b"fn on_start(ctx) { ctx.set_scale(ctx.entity(), 1e-40, 1e-40, 1e-40); }",
    )?;
    publish(&editor, &mut assets)?;
    let mut owner = EntityData::new("bad initialization");
    owner.model = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find_map(|entity| entity.model.clone());
    assert!(owner.model.is_some());
    let id = owner.id;
    editor.document.insert_entity(owner, &mut assets)?;
    editor.edit_script(id, Some(Script::new(script)), &mut assets)?;
    editor.commit()?;
    let before = editor.document.shared_source();
    let history = editor.document.history_len();
    let bytes = std::fs::read(&editor.source)?;
    assert!(editor.start(&mut assets).is_err());
    assert!(editor.play.is_none());
    assert!(Arc::ptr_eq(&before, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), history);
    assert_eq!(std::fs::read(&editor.source)?, bytes);
    Ok(())
}
