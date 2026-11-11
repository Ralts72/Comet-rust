use super::*;
use comet_core::EntityId;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Vec2};

fn fixture() -> Result<(tempfile::TempDir, Editor, AssetStore, EntityId)> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let file = editor.project_root().join("assets/test.rhai");
    std::fs::write(&file, "fn on_fixed_update(ctx, dt) {}")?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let asset = comet_asset_pipeline::SourceMeta::load(&file)?.id;
    editor.sources = comet_asset_pipeline::SourceCatalog::read(
        &comet_core::ProjectPaths::open(editor.project_root())?,
        assets.index(),
    )?;
    let mut entity = EntityData::new("script owner");
    entity.script = Some(Script::new(asset));
    let id = entity.id;
    editor.document = comet_editor_core::Document::open(comet_scene::SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    })?;
    Ok((temp, editor, assets, id))
}

fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    events: Vec<Event>,
) -> egui::FullOutput {
    let entity = editor
        .snapshot(assets)
        .unwrap()
        .entities
        .into_iter()
        .find(|e| e.script.is_some())
        .unwrap();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600., 1100.))),
            events,
            ..Default::default()
        },
        |ui| draw(editor, ui, assets, &entity),
    );
    output.textures_delta.clear();
    output
}

fn label(output: &egui::FullOutput, text: &str) -> Pos2 {
    fn find(shape: &egui::epaint::Shape, text: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(value) if value.galley.job.text == text => {
                Some(value.pos + value.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(values) => values.iter().find_map(|shape| find(shape, text)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, text))
        .unwrap_or_else(|| panic!("missing label {text}"))
}

fn button(position: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn click(
    ctx: &egui::Context,
    editor: &mut Editor,
    assets: &mut AssetStore,
    position: Pos2,
) -> egui::FullOutput {
    frame(
        ctx,
        editor,
        assets,
        vec![Event::PointerMoved(position), button(position, true)],
    );
    frame(ctx, editor, assets, vec![button(position, false)])
}

fn key(key: egui::Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn authored_entity_picker_persists_one_instance_and_runtime_controls_only_that_rig() -> Result<()> {
    use comet_scene::{PrefabInstance, PrefabResolver};
    let (_temp, mut editor, mut assets, owner) = fixture()?;
    std::fs::write(
        editor.project_root().join("assets/test.rhai"),
        "fn on_start(ctx) { ctx.pause_animation(ctx.param(\"animator\")); }",
    )?;
    let prefab = crate::test_support::prepare_skin_asset(&editor)?;
    assets.refresh()?;
    let rig = assets.prefab(prefab)?;
    let controller = rig
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .unwrap()
        .id;
    let first = comet_core::InstanceId::new();
    let second = comet_core::InstanceId::new();
    let mut source = editor.document.source();
    source.base.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .insert("animator".into(), ScriptValue::Entity(None));
    source.instances = [first, second]
        .into_iter()
        .map(|id| PrefabInstance {
            translation: [0.; 3],
            id,
            prefab,
            parent: None,
            overrides: vec![],
        })
        .collect();
    editor.document = comet_editor_core::Document::open_source(source)?;
    let first = editor.document.shared_source().instances[0].id;
    let second = editor.document.shared_source().instances[1].id;
    let original = editor.document.shared_source();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    let references = editor.cached_field_references(&ctx, owner, &mut assets)?;
    let target = controller.in_instance(first);
    let name = references
        .targets
        .iter()
        .find(|(id, _)| *id == target)
        .unwrap()
        .1
        .clone();
    assert!(!name.contains(&target.to_string()));
    click(&ctx, &mut editor, &mut assets, label(&output, "无引用"));
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    click(&ctx, &mut editor, &mut assets, label(&output, &name));
    assert!(editor.error.is_none(), "{:?}", editor.error);
    let changed = editor.document.shared_source();
    assert_eq!(
        changed.base.entities[0].script.as_ref().unwrap().parameters["animator"],
        ScriptValue::Entity(Some(EntityRef::InInstance {
            instance: first,
            entity: controller
        }))
    );
    assert_eq!(
        changed.base.version,
        comet_scene::SceneSnapshot::default().version
    );
    assert_eq!(editor.document.history_len(), 1);
    let snapshot = editor.snapshot(&mut assets)?;
    assert_eq!(
        snapshot
            .entities
            .iter()
            .find(|entity| entity.id == owner)
            .unwrap()
            .script
            .as_ref()
            .unwrap()
            .parameters["animator"],
        ScriptValue::Entity(Some(EntityRef::local(target)))
    );
    editor.save(&mut assets)?;
    assert_eq!(
        comet_editor_core::Document::load(&editor.source)?.shared_source(),
        changed
    );
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), original);
    editor.document.redo()?;
    assert_eq!(editor.document.shared_source(), changed);

    // The shared selector cache owns labels and IDs, not the prefab product or expanded world.
    frame(&ctx, &mut editor, &mut assets, vec![]);
    let cache = editor.cached_field_references(&ctx, owner, &mut assets)?;
    for id in assets.index().products.keys().copied().collect::<Vec<_>>() {
        assets.evict(id);
    }
    for _ in 0..4 {
        frame(&ctx, &mut editor, &mut assets, vec![]);
    }
    assert_eq!(assets.resident_count(), 0);
    assert!(std::sync::Arc::ptr_eq(
        &cache,
        &editor.cached_field_references(&ctx, owner, &mut assets)?
    ));

    editor.start(&mut assets)?;
    let play = editor.play.as_ref().unwrap();
    assert!(!play.script_diagnostics().iter().any(|entry| entry.failed));
    assert!(!play.animation_state(target).unwrap().playing);
    assert!(
        play.animation_state(controller.in_instance(second))
            .unwrap()
            .playing
    );
    editor.stop();
    assert_eq!(editor.document.shared_source(), changed);
    Ok(())
}

#[test]
fn number_drag_commits_once_and_escape_cancels_a_text_preview() -> Result<()> {
    let (_temp, mut editor, mut assets, id) = fixture()?;
    let mut source = editor.document.source();
    source.base.entities[0]
        .script
        .as_mut()
        .unwrap()
        .parameters
        .extend([
            ("number".into(), ScriptValue::Number(1.25)),
            ("text".into(), ScriptValue::Text("before".into())),
        ]);
    editor.document = comet_editor_core::Document::open_source(source)?;
    let baseline = editor.document.shared_source();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    let start = label(&output, "1.25");
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![Event::PointerMoved(start + Vec2::new(30., 0.))],
    );
    assert!(editor.document.is_editing());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![Event::PointerMoved(start + Vec2::new(55., 0.))],
    );
    let output = frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![button(start + Vec2::new(55., 0.), false)],
    );
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.history_len(), 1);
    let changed = editor.document.shared_source();
    assert_ne!(changed, baseline);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), baseline);
    editor.document.redo()?;
    click(&ctx, &mut editor, &mut assets, label(&output, "before"));
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![Event::Text("after".into())],
    );
    assert!(editor.document.is_editing());
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![key(egui::Key::Escape, Modifiers::NONE)],
    );
    assert!(!editor.document.is_editing());
    assert_eq!(editor.document.shared_source(), changed);
    assert_eq!(editor.document.history_len(), 1);
    assert!(
        editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .any(|e| e.id == id)
    );
    Ok(())
}

#[test]
fn adding_renaming_and_removing_parameters_publish_distinct_undo_steps() -> Result<()> {
    let (_temp, mut editor, mut assets, _id) = fixture()?;
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    click(&ctx, &mut editor, &mut assets, label(&output, "新参数名"));
    let output = frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![Event::Text("counter".into())],
    );
    click(&ctx, &mut editor, &mut assets, label(&output, "添加参数"));
    assert_eq!(editor.document.history_len(), 1);
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    click(&ctx, &mut editor, &mut assets, label(&output, "重命名"));
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    click(&ctx, &mut editor, &mut assets, label(&output, "counter"));
    let output = frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![
            key(egui::Key::A, Modifiers::COMMAND),
            Event::Text("renamed".into()),
        ],
    );
    assert!(
        editor.document.shared_source().base.entities[0]
            .script
            .as_ref()
            .unwrap()
            .parameters
            .contains_key("counter")
    );
    frame(
        &ctx,
        &mut editor,
        &mut assets,
        vec![key(egui::Key::Enter, Modifiers::NONE)],
    );
    assert_eq!(
        editor.document.history_len(),
        1,
        "losing text focus never applies a name draft"
    );
    click(&ctx, &mut editor, &mut assets, label(&output, "确认重命名"));
    assert_eq!(editor.document.history_len(), 2);
    let renamed = editor.document.shared_source();
    assert!(
        renamed.base.entities[0]
            .script
            .as_ref()
            .unwrap()
            .parameters
            .contains_key("renamed")
    );
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    click(&ctx, &mut editor, &mut assets, label(&output, "删除参数"));
    assert!(
        editor.document.shared_source().base.entities[0]
            .script
            .as_ref()
            .unwrap()
            .parameters
            .is_empty()
    );
    assert_eq!(editor.document.history_len(), 3);
    editor.document.undo()?;
    assert_eq!(editor.document.shared_source(), renamed);
    Ok(())
}

#[test]
fn unconfirmed_parameter_names_never_follow_save_focus_loss_cancel_or_close() -> Result<()> {
    for action in ["cancel", "escape", "close"] {
        let (_temp, mut editor, mut assets, _id) = fixture()?;
        let mut source = editor.document.source();
        source.base.entities[0]
            .script
            .as_mut()
            .unwrap()
            .parameters
            .insert("counter".into(), ScriptValue::Integer(1));
        editor.document = comet_editor_core::Document::open_source(source)?;
        editor.save(&mut assets)?;
        let baseline = editor.document.shared_source();
        let saved = std::fs::read(&editor.source)?;
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        frame(&ctx, &mut editor, &mut assets, vec![]);
        let output = frame(&ctx, &mut editor, &mut assets, vec![]);
        click(&ctx, &mut editor, &mut assets, label(&output, "重命名"));
        let output = frame(&ctx, &mut editor, &mut assets, vec![]);
        click(&ctx, &mut editor, &mut assets, label(&output, "counter"));
        frame(
            &ctx,
            &mut editor,
            &mut assets,
            vec![
                key(egui::Key::A, Modifiers::COMMAND),
                Event::Text("unconfirmed".into()),
            ],
        );
        assert_eq!(editor.document.shared_source(), baseline);
        assert!(!editor.document.is_editing());
        // Toolbar save runs before the inspector processes the corresponding focus loss.
        editor.save(&mut assets)?;
        let output = frame(
            &ctx,
            &mut editor,
            &mut assets,
            vec![key(egui::Key::Enter, Modifiers::NONE)],
        );
        assert_eq!(editor.document.shared_source(), baseline);
        assert!(!editor.document.is_dirty());
        if action == "close" {
            assert!(editor.request_editor_close());
            assert_eq!(editor.document.shared_source(), baseline);
            assert_eq!(std::fs::read(&editor.source)?, saved);
            continue;
        }
        if action == "escape" {
            frame(
                &ctx,
                &mut editor,
                &mut assets,
                vec![key(egui::Key::Escape, Modifiers::NONE)],
            );
        } else {
            click(&ctx, &mut editor, &mut assets, label(&output, "取消重命名"));
        }
        frame(&ctx, &mut editor, &mut assets, vec![]);
        assert_eq!(editor.document.shared_source(), baseline);
        assert_eq!(editor.document.history_len(), 0);
        assert_eq!(std::fs::read(&editor.source)?, saved);
        assert!(editor.request_editor_close());
        assert_eq!(editor.document.shared_source(), baseline);
    }
    Ok(())
}

#[test]
fn script_controls_are_read_only_during_play() -> Result<()> {
    let (_temp, mut editor, mut assets, _id) = fixture()?;
    let authored = editor.document.shared_source();
    editor.start(&mut assets)?;
    let world = editor.world();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame(&ctx, &mut editor, &mut assets, vec![]);
    let output = frame(&ctx, &mut editor, &mut assets, vec![]);
    click(&ctx, &mut editor, &mut assets, label(&output, "启用脚本"));
    assert_eq!(editor.world(), world);
    assert_eq!(editor.document.shared_source(), authored);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}
