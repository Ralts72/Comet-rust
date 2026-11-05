use super::*;
use anyhow::Result;
use comet_core::{AssetId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{
    BodyMotion, ContentSource, EntityData, FieldOverride, HingeJoint, JointMotor, PhysicsBody,
    PrefabInstance, SceneSnapshot, schema::FieldValue,
};
use egui::{Event, PointerButton, Pos2, Vec2};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    prefab: AssetId,
    prefab_path: PathBuf,
    prefab_bytes: Vec<u8>,
    instance: InstanceId,
    second: InstanceId,
    source_entity: EntityId,
    source_other: EntityId,
    selected: EntityId,
}

fn fixture() -> Result<Fixture> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let mut entity = EntityData::new("可恢复对象");
    entity.transform.translation = [1., 2., 3.];
    entity.physics = Some(PhysicsBody {
        motion: BodyMotion::Dynamic,
        ..Default::default()
    });
    entity.hinge_joint = Some(HingeJoint::default());
    let source_entity = entity.id;
    let other = EntityData::new("其他对象");
    let source_other = other.id;
    let source = ContentSource {
        base: SceneSnapshot {
            entities: vec![entity, other],
            ..Default::default()
        },
        ..Default::default()
    };
    let prefab_path = editor.project_root().join("assets/restorable.prefab");
    let prefab_bytes = source.to_json()?;
    fs::write(&prefab_path, &prefab_bytes)?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    let prefab = comet_asset_pipeline::SourceMeta::load(&prefab_path)?.id;
    let instance = InstanceId::new();
    let second = InstanceId::new();
    let field = |entity, field, value| FieldOverride {
        entity,
        field,
        value,
    };
    editor.document = Document::open_source(ContentSource {
        instances: vec![
            PrefabInstance {
                id: instance,
                prefab,
                parent: None,
                overrides: vec![
                    field(
                        source_entity,
                        schema::TRANSLATION,
                        FieldValue::Vec3([1., 2., 3.]),
                    ),
                    field(source_entity, schema::SCALE, FieldValue::Vec3([2.; 3])),
                    field(
                        source_entity,
                        schema::HINGE_MOTOR,
                        FieldValue::JointMotor(Some(JointMotor::Velocity {
                            speed: 2.,
                            max_effort: 3.,
                        })),
                    ),
                    field(
                        source_other,
                        schema::TRANSLATION,
                        FieldValue::Vec3([7., 8., 9.]),
                    ),
                ],
            },
            PrefabInstance {
                id: second,
                prefab,
                parent: None,
                overrides: vec![field(
                    source_entity,
                    schema::TRANSLATION,
                    FieldValue::Vec3([12., 3., 4.]),
                )],
            },
        ],
        ..Default::default()
    })?;
    Ok(Fixture {
        _temp: temp,
        editor,
        assets,
        prefab,
        prefab_path,
        prefab_bytes,
        instance,
        second,
        source_entity,
        source_other,
        selected: source_entity.in_instance(instance),
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
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(640., 800.))),
            events,
            ..Default::default()
        },
        |ui| draw(&mut f.editor, ui, &mut f.assets, f.selected),
    );
    output.textures_delta.clear();
    output
}

fn labels(output: &egui::FullOutput, text: &str) -> Vec<Pos2> {
    fn find(shape: &egui::epaint::Shape, text: &str, positions: &mut Vec<Pos2>) {
        match shape {
            egui::epaint::Shape::Text(value) if value.galley.job.text.starts_with(text) => {
                positions.push(value.pos + value.galley.rect.center().to_vec2());
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    find(shape, text, positions);
                }
            }
            _ => {}
        }
    }
    let mut positions = vec![];
    for shape in &output.shapes {
        find(&shape.shape, text, &mut positions);
    }
    positions
}

fn label(output: &egui::FullOutput, text: &str) -> Pos2 {
    *labels(output, text)
        .first()
        .unwrap_or_else(|| panic!("missing label {text}"))
}

fn click(
    ctx: &egui::Context,
    f: &mut Fixture,
    pos: Pos2,
    button: PointerButton,
) -> egui::FullOutput {
    let event = |pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Default::default(),
    };
    frame(ctx, f, vec![Event::PointerMoved(pos), event(true)]);
    frame(ctx, f, vec![event(false)])
}

fn expanded(ctx: &egui::Context, f: &mut Fixture) -> egui::FullOutput {
    let output = frame(ctx, f, vec![]);
    assert!(
        labels(&output, "恢复继承").is_empty(),
        "the section starts collapsed"
    );
    click(
        ctx,
        f,
        label(&output, "实例覆盖 (3)"),
        PointerButton::Primary,
    )
}

fn fields(f: &Fixture, instance: InstanceId, entity: EntityId) -> Vec<FieldKey> {
    f.editor
        .document
        .shared_source()
        .instances
        .iter()
        .find(|value| value.id == instance)
        .unwrap()
        .overrides
        .iter()
        .filter(|value| value.entity == entity)
        .map(|value| value.field)
        .collect()
}

#[test]
fn explicit_pin_and_structured_overrides_restore_through_widgets_with_undo_redo_and_save()
-> Result<()> {
    let mut f = fixture()?;
    let original = f.editor.document.shared_source();
    let second = original
        .instances
        .iter()
        .find(|value| value.id == f.second)
        .unwrap()
        .clone();
    let other = fields(&f, f.instance, f.source_other);
    let ctx = context();
    let output = expanded(&ctx, &mut f);
    assert!(!labels(&output, "铰链关节 · 马达").is_empty());
    let row = label(&output, "变换 · 位置");
    let restore = labels(&output, "恢复继承")
        .into_iter()
        .find(|point| (point.y - row.y).abs() < 1.)
        .unwrap();
    click(&ctx, &mut f, restore, PointerButton::Secondary);
    assert!(Arc::ptr_eq(&original, &f.editor.document.shared_source()));
    for _ in 0..10 {
        frame(&ctx, &mut f, vec![]);
    }
    assert!(
        Arc::ptr_eq(&original, &f.editor.document.shared_source()),
        "idle inspection must preserve an explicit pin equal to the source"
    );
    click(&ctx, &mut f, restore, PointerButton::Primary);
    assert_eq!(
        fields(&f, f.instance, f.source_entity),
        [schema::SCALE, schema::HINGE_MOTOR]
    );
    assert_eq!(fields(&f, f.instance, f.source_other), other);
    assert_eq!(f.editor.document.history_len(), 1);
    let one = f.editor.document.shared_source();
    assert_eq!(
        one.instances
            .iter()
            .find(|value| value.id == f.second)
            .unwrap(),
        &second
    );
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), one);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), one);
    assert!(!f.editor.document.is_dirty());
    let output = frame(&ctx, &mut f, vec![]);
    assert!(!labels(&output, "实例覆盖 (2)").is_empty());
    click(
        &ctx,
        &mut f,
        label(&output, "恢复全部继承"),
        PointerButton::Primary,
    );
    assert!(fields(&f, f.instance, f.source_entity).is_empty());
    assert_eq!(fields(&f, f.instance, f.source_other), other);
    assert_eq!(
        f.editor.document.history_len(),
        2,
        "all fields form one undo entry"
    );
    let all = f.editor.document.shared_source();
    assert_eq!(
        all.instances
            .iter()
            .find(|value| value.id == f.second)
            .unwrap(),
        &second
    );
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), one);
    assert!(!f.editor.document.is_dirty());
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), all);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), all);
    assert_eq!(fs::read(&f.prefab_path)?, f.prefab_bytes);
    Ok(())
}

#[test]
fn play_hides_author_overrides_and_background_work_disables_the_restore_buttons() -> Result<()> {
    let mut f = fixture()?;
    let ctx = context();
    let output = expanded(&ctx, &mut f);
    let restore = label(&output, "恢复全部继承");
    let source = f.editor.document.shared_source();
    f.editor.start(&mut f.assets)?;
    let output = click(&ctx, &mut f, restore, PointerButton::Primary);
    assert!(labels(&output, "实例覆盖").is_empty());
    assert!(labels(&output, "恢复").is_empty());
    assert!(Arc::ptr_eq(&source, &f.editor.document.shared_source()));
    f.editor.stop();
    let paths = comet_core::ProjectPaths::open(f.editor.project_root())?;
    f.editor.resource_build.start(
        &paths,
        &f.editor.products,
        f.editor.document.world_id(),
        f.editor.document.revision(),
        f.editor.document.retained_sources().collect(),
        f.editor.document.prefab_asset(),
    )?;
    assert!(f.editor.is_busy());
    let output = frame(&ctx, &mut f, vec![]);
    assert!(!labels(&output, "恢复全部继承").is_empty());
    click(
        &ctx,
        &mut f,
        label(&output, "恢复全部继承"),
        PointerButton::Primary,
    );
    assert!(Arc::ptr_eq(&source, &f.editor.document.shared_source()));
    assert_eq!(f.editor.document.history_len(), 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while f.editor.resource_build.is_running() {
        if let Some(result) = f.editor.resource_build.poll() {
            result.map_err(anyhow::Error::msg)?;
        }
        anyhow::ensure!(Instant::now() < deadline, "resource build timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(fs::read(&f.prefab_path)?, f.prefab_bytes);
    Ok(())
}

#[test]
fn idle_inspection_reuses_one_small_cache_and_recomputes_after_identity_or_version_changes()
-> Result<()> {
    let mut f = fixture()?;
    let ctx = context();
    frame(&ctx, &mut f, vec![]);
    f.assets.evict(f.prefab);
    let baseline = f.assets.resident_count();
    for _ in 0..20 {
        frame(&ctx, &mut f, vec![]);
    }
    assert_eq!(
        f.assets.resident_count(),
        baseline,
        "idle UI must not resolve and clone the prefab again"
    );
    f.selected = f.source_entity.in_instance(f.second);
    let output = frame(&ctx, &mut f, vec![]);
    assert!(!labels(&output, "实例覆盖 (1)").is_empty());
    assert_eq!(f.assets.resident_count(), baseline + 1);
    f.assets.evict(f.prefab);
    f.selected = f.source_entity.in_instance(f.instance);
    frame(&ctx, &mut f, vec![]);
    assert_eq!(
        f.assets.resident_count(),
        baseline + 1,
        "selection replaces the single cache entry"
    );
    f.editor.document = Document::open_source(f.editor.document.source())?;
    f.assets.evict(f.prefab);
    frame(&ctx, &mut f, vec![]);
    assert_eq!(
        f.assets.resident_count(),
        baseline + 1,
        "new document identity invalidates cached targets"
    );
    f.editor
        .revert_prefab_fields(f.selected, &[schema::SCALE], &mut f.assets)?;
    f.assets.evict(f.prefab);
    let output = frame(&ctx, &mut f, vec![]);
    assert!(!labels(&output, "实例覆盖 (2)").is_empty());
    assert_eq!(
        f.assets.resident_count(),
        baseline + 1,
        "content revisions invalidate cached field keys"
    );
    let mut changed = ContentSource::from_json(&f.prefab_bytes)?;
    changed
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.source_entity)
        .unwrap()
        .transform
        .translation = [5., 6., 7.];
    fs::write(&f.prefab_path, changed.to_json()?)?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    assert_eq!(f.assets.resident_count(), baseline);
    frame(&ctx, &mut f, vec![]);
    assert_eq!(
        f.assets.resident_count(),
        baseline + 1,
        "accepted product versions invalidate the cache"
    );
    assert!(fields(&f, f.instance, f.source_entity).contains(&schema::TRANSLATION));
    Ok(())
}

#[test]
fn authored_and_nested_entities_do_not_offer_direct_instance_restore_actions() -> Result<()> {
    let mut f = fixture()?;
    let authored = EntityData::new("作者对象");
    f.selected = authored.id;
    f.editor.document.insert_entity(authored, &mut f.assets)?;
    let ctx = context();
    let output = frame(&ctx, &mut f, vec![]);
    assert!(labels(&output, "实例覆盖").is_empty());
    let inner = InstanceId::new();
    let nested = ContentSource {
        instances: vec![PrefabInstance {
            id: inner,
            prefab: f.prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    let path = f.editor.project_root().join("assets/nested.prefab");
    fs::write(&path, nested.to_json()?)?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let outer = InstanceId::new();
    f.editor.document = Document::open_source(ContentSource {
        instances: vec![PrefabInstance {
            id: outer,
            prefab: comet_asset_pipeline::SourceMeta::load(&path)?.id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = f.source_entity.in_instance(inner.in_instance(outer));
    assert!(
        f.editor
            .snapshot(&mut f.assets)?
            .entities
            .iter()
            .any(|entity| entity.id == f.selected)
    );
    let output = frame(&ctx, &mut f, vec![]);
    assert!(labels(&output, "实例覆盖").is_empty());
    assert!(!labels(&output, "嵌套实例的覆盖").is_empty());
    assert!(labels(&output, "恢复").is_empty());
    Ok(())
}
