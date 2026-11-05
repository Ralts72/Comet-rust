use super::*;
use anyhow::Result;
use comet_asset_data::product::Product;
use comet_editor_core::Document;
use comet_scene::{ContentSource, PrefabInstance, SceneSnapshot};
use egui::{Event, PointerButton, Pos2, Rect, Vec2};
use std::fs;

struct Fixture {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    selected: EntityId,
}

fn fixture() -> Result<Fixture> {
    let (temp, mut editor, assets) = crate::test_support::fixture()?;
    let entity = EntityData::new("作者模型对象");
    let selected = entity.id;
    editor.document = Document::open(SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    })?;
    Ok(Fixture {
        _temp: temp,
        editor,
        assets,
        selected,
    })
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| style.animation_time = 0.);
    }
    ctx
}

fn frame(ctx: &egui::Context, f: &mut Fixture, events: Vec<Event>) -> egui::FullOutput {
    let entity = f
        .editor
        .snapshot(&mut f.assets)
        .unwrap()
        .entities
        .into_iter()
        .find(|entity| entity.id == f.selected)
        .unwrap();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 900.))),
            events,
            ..Default::default()
        },
        |ui| draw(&mut f.editor, ui, &mut f.assets, &entity),
    );
    output.textures_delta.clear();
    output
}

fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn find(shape: &egui::epaint::Shape, result: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => result.push((
                text.galley.job.text.clone(),
                text.pos + text.galley.rect.center().to_vec2(),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    find(shape, result);
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        find(&shape.shape, &mut result);
    }
    result
}

fn label(output: &egui::FullOutput, text: &str) -> Pos2 {
    texts(output)
        .into_iter()
        .find(|(value, _)| value.contains(text))
        .map(|(_, position)| position)
        .unwrap_or_else(|| panic!("missing label {text}; {:?}", texts(output)))
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

fn click(ctx: &egui::Context, f: &mut Fixture, position: Pos2) -> egui::FullOutput {
    frame(
        ctx,
        f,
        vec![Event::PointerMoved(position), button(position, true)],
    );
    frame(ctx, f, vec![button(position, false)])
}

fn combo(output: &egui::FullOutput, field: &str) -> Pos2 {
    let row = texts(output)
        .into_iter()
        .find(|(text, _)| text == field)
        .unwrap()
        .1;
    texts(output)
        .into_iter()
        .find(|(_, point)| (point.y - row.y).abs() < 2. && point.x > row.x + 20.)
        .map(|(_, point)| point)
        .unwrap_or_else(|| panic!("missing {field} selector"))
}

fn select(ctx: &egui::Context, f: &mut Fixture, field: &str, next: &str) {
    let output = frame(ctx, f, vec![]);
    let anchor = combo(&output, field);
    click(ctx, f, anchor);
    // With one egui pass, a newly opened popup paints after its sizing frame.
    let output = frame(ctx, f, vec![]);
    let position = texts(&output)
        .into_iter()
        .find(|(text, point)| text.contains(next) && point.y > anchor.y + 3.)
        .map(|(_, point)| point)
        .unwrap_or_else(|| panic!("missing option {next}; {:?}", texts(&output)));
    click(ctx, f, position);
}

fn configure_draft(ctx: &egui::Context, f: &mut Fixture) {
    select(ctx, f, "网格", "AmberTwoMaterials");
    select(ctx, f, "材质", "Porcelain");
}

fn model(f: &mut Fixture) -> Option<Model> {
    f.editor
        .snapshot(&mut f.assets)
        .unwrap()
        .entities
        .into_iter()
        .find(|entity| entity.id == f.selected)
        .unwrap()
        .model
}

fn press_add(ctx: &egui::Context, f: &mut Fixture) {
    let output = frame(ctx, f, vec![]);
    click(ctx, f, label(&output, "添加模型"));
}

#[test]
fn authored_model_widgets_keep_drafts_private_and_preserve_roughness_when_switching_assets()
-> Result<()> {
    let mut f = fixture()?;
    let ctx = context();
    let original = f.editor.document.shared_source();
    configure_draft(&ctx, &mut f);
    assert!(Arc::ptr_eq(&original, &f.editor.document.shared_source()));
    assert!(!f.editor.document.is_dirty());
    f.editor.start(&mut f.assets)?;
    frame(&ctx, &mut f, vec![]);
    f.editor.stop();
    let output = frame(&ctx, &mut f, vec![]);
    assert!(texts(&output).iter().any(|(text, _)| text == "选择网格"));
    configure_draft(&ctx, &mut f);
    frame(
        &ctx,
        &mut f,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    let output = frame(&ctx, &mut f, vec![]);
    assert!(texts(&output).iter().any(|(text, _)| text == "选择网格"));
    assert!(texts(&output).iter().any(|(text, _)| text == "选择材质"));
    configure_draft(&ctx, &mut f);
    press_add(&ctx, &mut f);
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let added = model(&mut f).unwrap();
    let Product::Material(material) = &f.assets.load(added.surface.material)?.payload else {
        panic!("expected selected material");
    };
    assert_eq!(added.surface.roughness, material.roughness);
    assert_eq!(f.editor.document.history_len(), 1);
    assert_eq!(
        comet_application::extract(&f.editor.snapshot(&mut f.assets)?, &mut f.assets, &[])?.len(),
        1
    );
    f.editor.edit(
        f.selected,
        schema::ROUGHNESS,
        FieldValue::Scalar(0.),
        &mut f.assets,
    )?;
    f.editor.commit()?;
    let material = model(&mut f).unwrap().surface.material;
    select(&ctx, &mut f, "网格", "BlueTwoMaterials");
    let changed_mesh = model(&mut f).unwrap();
    assert_ne!(changed_mesh.mesh, added.mesh);
    assert_eq!(changed_mesh.surface.material, material);
    assert_eq!(changed_mesh.surface.roughness, 0.);
    select(&ctx, &mut f, "材质", "Blue");
    let changed = model(&mut f).unwrap();
    assert_eq!(changed.mesh, changed_mesh.mesh);
    assert_ne!(changed.surface.material, material);
    assert_eq!(changed.surface.roughness, 0.);
    assert_eq!(f.editor.document.history_len(), 4);
    let source = f.editor.document.shared_source();
    select(&ctx, &mut f, "网格", "BlueTwoMaterials");
    assert!(Arc::ptr_eq(&source, &f.editor.document.shared_source()));
    assert_eq!(f.editor.document.history_len(), 4);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), source);
    let output = frame(&ctx, &mut f, vec![]);
    click(&ctx, &mut f, label(&output, "移除模型"));
    assert!(model(&mut f).is_none());
    assert_eq!(f.editor.document.history_len(), 5);
    f.editor.document.undo()?;
    assert_eq!(model(&mut f), Some(changed));
    assert!(!f.editor.document.is_dirty());
    f.editor.document.redo()?;
    assert!(model(&mut f).is_none());
    Ok(())
}

fn slider(output: &egui::FullOutput) -> Pos2 {
    let y = label(output, "粗糙度").y;
    fn find(shape: &egui::epaint::Shape, y: f32, rects: &mut Vec<Rect>) {
        match shape {
            egui::epaint::Shape::Rect(rect)
                if (rect.rect.center().y - y).abs() < 2. && rect.rect.width() > 30. =>
            {
                rects.push(rect.rect)
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    find(shape, y, rects);
                }
            }
            _ => {}
        }
    }
    let mut rects = vec![];
    for shape in &output.shapes {
        find(&shape.shape, y, &mut rects);
    }
    rects
        .into_iter()
        .max_by(|a, b| a.width().total_cmp(&b.width()))
        .expect("roughness track")
        .center()
}

#[test]
fn roughness_drag_uses_one_transaction_and_escape_restores_the_previous_model() -> Result<()> {
    let mut f = fixture()?;
    let ctx = context();
    configure_draft(&ctx, &mut f);
    press_add(&ctx, &mut f);
    let history = f.editor.document.history_len();
    let original = f.editor.document.shared_source();
    let output = frame(&ctx, &mut f, vec![]);
    let start = slider(&output);
    frame(
        &ctx,
        &mut f,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut f,
        vec![Event::PointerMoved(start + Vec2::new(20., 0.))],
    );
    assert!(f.editor.document.is_editing());
    assert_ne!(f.editor.document.shared_source(), original);
    frame(
        &ctx,
        &mut f,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), history);
    frame(&ctx, &mut f, vec![button(start, false)]);
    let output = frame(&ctx, &mut f, vec![]);
    let start = slider(&output);
    frame(
        &ctx,
        &mut f,
        vec![Event::PointerMoved(start), button(start, true)],
    );
    frame(
        &ctx,
        &mut f,
        vec![Event::PointerMoved(start + Vec2::new(25., 0.))],
    );
    frame(
        &ctx,
        &mut f,
        vec![button(start + Vec2::new(25., 0.), false)],
    );
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), history + 1);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn metadata_choices_do_not_load_products_and_change_only_with_the_accepted_index() -> Result<()> {
    let mut f = fixture()?;
    let ctx = context();
    let ids: Vec<_> = f.assets.index().products.keys().copied().collect();
    for id in ids {
        f.assets.evict(id);
    }
    assert_eq!(f.assets.resident_count(), 0);
    for _ in 0..10 {
        frame(&ctx, &mut f, vec![]);
    }
    configure_draft(&ctx, &mut f);
    assert_eq!(
        f.assets.resident_count(),
        0,
        "opening and choosing metadata must not load mesh or texture payloads"
    );
    let directory = f.editor.project_root().join("assets");
    fs::rename(
        directory.join("showcase.gltf"),
        directory.join("renamed.gltf"),
    )?;
    fs::rename(
        directory.join("showcase.gltf.meta"),
        directory.join("renamed.gltf.meta"),
    )?;
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text.contains("showcase.gltf") && text.contains("AmberTwoMaterials"))
    );
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text.contains("renamed.gltf"))
    );
    assert_eq!(f.assets.resident_count(), 0);
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output).iter().any(|(text, _)| text == "选择网格"),
        "accepted index changes discard stale drafts"
    );
    let anchor = label(&output, "选择网格");
    click(&ctx, &mut f, anchor);
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text.contains("renamed.gltf") && text.contains("AmberTwoMaterials"))
    );
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text.contains("showcase.gltf"))
    );
    assert_eq!(f.assets.resident_count(), 0);
    // Optional editor metadata is not required by the runtime product protocol.
    let mesh = f
        .assets
        .index()
        .products
        .iter()
        .find(|(_, entry)| entry.kind == AssetKind::Mesh)
        .map(|(id, _)| *id)
        .unwrap();
    let index_path = f.editor.products.join("index.json");
    let mut index: serde_json::Value = serde_json::from_slice(&fs::read(&index_path)?)?;
    index["products"][mesh.to_string()]["editor"] = serde_json::Value::Null;
    fs::write(index_path, serde_json::to_vec(&index)?)?;
    f.assets.refresh()?;
    frame(
        &ctx,
        &mut f,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    let output = frame(&ctx, &mut f, vec![]);
    click(&ctx, &mut f, label(&output, "选择网格"));
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text.starts_with("未命名网格"))
    );
    assert!(
        texts(&output)
            .iter()
            .all(|(text, _)| !text.contains(&mesh.to_string()))
    );
    assert!(!f.editor.document.is_dirty());
    Ok(())
}

#[test]
fn prefab_asset_choices_only_override_changed_fields_and_play_or_nested_models_are_read_only()
-> Result<()> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let snapshot = editor.snapshot(&mut assets)?;
    let selected = snapshot
        .entities
        .iter()
        .find(|entity| entity.name.starts_with("Amber cube /") && entity.model.is_some())
        .unwrap()
        .id;
    let FieldTarget::Prefab {
        instance,
        entity: source_entity,
    } = editor.resolve_field_target(selected, &mut assets)?
    else {
        panic!("expected imported prefab");
    };
    let root = editor
        .document
        .shared_source()
        .instances
        .iter()
        .find(|value| value.id == instance)
        .unwrap()
        .prefab;
    let source_path = editor.project_root().join("assets/showcase.gltf");
    let bytes = fs::read(&source_path)?;
    let mut f = Fixture {
        _temp: temp,
        editor,
        assets,
        selected,
    };
    let ctx = context();
    let before = model(&mut f).unwrap();
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text.contains("移除模型"))
    );
    select(&ctx, &mut f, "材质", "Porcelain");
    let next = model(&mut f).unwrap();
    assert_eq!(next.mesh, before.mesh);
    assert_eq!(next.surface.roughness, before.surface.roughness);
    assert_ne!(next.surface.material, before.surface.material);
    let source = f.editor.document.shared_source();
    let overrides = &source
        .instances
        .iter()
        .find(|value| value.id == instance)
        .unwrap()
        .overrides;
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].entity, source_entity);
    assert_eq!(overrides[0].field, schema::MATERIAL);
    assert_eq!(overrides[0].value, FieldValue::Asset(next.surface.material));
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), source);
    f.editor.document.undo()?;
    assert_eq!(model(&mut f), Some(before));
    f.editor.document.redo()?;
    assert_eq!(model(&mut f), Some(next));
    f.editor.start(&mut f.assets)?;
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "运行期间模型配置只读。")
    );
    click(&ctx, &mut f, combo(&output, "材质"));
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text.contains("Ground"))
    );
    assert!(Arc::ptr_eq(&source, &f.editor.document.shared_source()));
    f.editor.stop();
    let inner = comet_core::InstanceId::new();
    let nested = ContentSource {
        instances: vec![PrefabInstance {
            id: inner,
            prefab: root,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    let path = f.editor.project_root().join("assets/nested.prefab");
    fs::write(&path, nested.to_json()?)?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let outer = comet_core::InstanceId::new();
    f.editor.document = Document::open_source(ContentSource {
        instances: vec![PrefabInstance {
            id: outer,
            prefab: comet_asset_pipeline::SourceMeta::load(&path)?.id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = source_entity.in_instance(inner.in_instance(outer));
    let nested = f.editor.document.shared_source();
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "嵌套实例的模型请在源 Prefab 中编辑。")
    );
    click(&ctx, &mut f, combo(&output, "材质"));
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text.contains("Porcelain"))
    );
    assert!(Arc::ptr_eq(&nested, &f.editor.document.shared_source()));
    assert_eq!(f.editor.document.history_len(), 0);
    assert_eq!(fs::read(source_path)?, bytes);
    Ok(())
}

#[test]
fn adding_on_an_unrepresentable_transform_fails_without_publishing_or_losing_the_draft()
-> Result<()> {
    let mut f = fixture()?;
    let mut snapshot = f.editor.document.snapshot();
    snapshot.entities[0].transform.scale = [1e-40; 3];
    f.editor.document = Document::open(snapshot)?;
    let original = f.editor.document.shared_source();
    let ctx = context();
    configure_draft(&ctx, &mut f);
    press_add(&ctx, &mut f);
    assert!(f.editor.error.is_some());
    assert!(Arc::ptr_eq(&original, &f.editor.document.shared_source()));
    assert_eq!(f.editor.document.history_len(), 0);
    assert!(!f.editor.document.is_editing());
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text.contains("AmberTwoMaterials"))
    );
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text.contains("Porcelain"))
    );
    assert!(model(&mut f).is_none());
    Ok(())
}

#[test]
fn a_project_without_render_assets_keeps_add_disabled_and_explains_how_to_prepare_them()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("empty");
    comet_asset_pipeline::create_project(&project)?;
    let (mut editor, assets) =
        crate::session::open(project, temp.path().join("products"), "main.scene".into())?;
    let entity = EntityData::new("empty model owner");
    let selected = entity.id;
    editor.document = Document::open(SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    })?;
    let mut f = Fixture {
        _temp: temp,
        editor,
        assets,
        selected,
    };
    let original = f.editor.document.shared_source();
    let ctx = context();
    let output = frame(&ctx, &mut f, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text.contains("导入含网格与材质的模型"))
    );
    click(&ctx, &mut f, label(&output, "添加模型"));
    assert!(model(&mut f).is_none());
    assert!(Arc::ptr_eq(&original, &f.editor.document.shared_source()));
    Ok(())
}
