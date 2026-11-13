use super::*;
use anyhow::{Context, Result};
use comet_editor_core::Document;
use comet_scene::{ContentSource, PrefabInstance, SceneSnapshot, schema};
use egui::{Event, PointerButton, Pos2, Rect, Vec2};
use std::fs;

struct Fixture {
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    prefab: AssetId,
    selected: EntityId,
    full: bool,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Result<Self> {
        let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
        let prefab = crate::test_support::prepare_skin_asset(&editor)?;
        assets.refresh()?;
        let source = comet_scene::PrefabResolver::prefab(&mut assets, prefab)?;
        let selected = source
            .base
            .entities
            .iter()
            .find(|entity| entity.animator.is_some())
            .context("imported controller")?
            .id;
        editor.document = Document::open_source(source)?;
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| style.animation_time = 0.);
        ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(1).unwrap());
        let mut value = Self {
            _temp: temp,
            editor,
            assets,
            ctx,
            prefab,
            selected,
            full: false,
        };
        value.frame(vec![]);
        value.frame(vec![]);
        Ok(value)
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        let snapshot = self.editor.snapshot(&mut self.assets).unwrap();
        let entity = snapshot
            .entities
            .iter()
            .find(|entity| entity.id == self.selected)
            .unwrap()
            .clone();
        self.editor.last_snapshot = Arc::new(snapshot);
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 1300.))),
                events,
                ..Default::default()
            },
            |ui| {
                if self.full {
                    super::super::draw(&mut self.editor, ui, &mut self.assets, &entity);
                } else {
                    draw(&mut self.editor, ui, &mut self.assets, &entity);
                }
            },
        );
        output.textures_delta.clear();
        output
    }
    fn click(&mut self, position: Pos2) -> egui::FullOutput {
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)])
    }
    fn press(&mut self, name: &str) -> egui::FullOutput {
        let output = self.frame(vec![]);
        self.click(label(&output, name))
    }
    fn choose(&mut self, current: &str, choice: &str) {
        let output = self.frame(vec![]);
        let anchor = label(&output, current);
        self.click(anchor);
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .find(|(text, position)| text == choice && position.y > anchor.y + 3.)
            .unwrap_or_else(|| panic!("missing option {choice}: {:?}", texts(&output)))
            .1;
        self.click(position);
    }
    fn source(&self) -> Arc<ContentSource> {
        self.editor.document.shared_source()
    }
    fn animator(&mut self) -> Animator {
        self.editor
            .snapshot(&mut self.assets)
            .unwrap()
            .entities
            .into_iter()
            .find(|entity| entity.id == self.selected)
            .unwrap()
            .animator
            .unwrap()
    }
    fn evict(&mut self) {
        for id in self
            .assets
            .index()
            .products
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            self.assets.evict(id);
        }
    }
}
fn button(position: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}
fn escape() -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn collect(shape: &egui::epaint::Shape, values: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => values.push((
                text.galley.text().to_owned(),
                text.pos + text.galley.rect.size() * 0.5,
            )),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, values);
                }
            }
            _ => {}
        }
    }
    let mut values = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, &mut values);
    }
    values
}
fn label(output: &egui::FullOutput, name: &str) -> Pos2 {
    texts(output)
        .into_iter()
        .find(|(text, _)| text == name)
        .unwrap_or_else(|| panic!("missing {name}: {:?}", texts(output)))
        .1
}
fn speed(output: &egui::FullOutput) -> Pos2 {
    texts(output)
        .into_iter()
        .find(|(text, _)| text.starts_with("播放速度 "))
        .unwrap()
        .1
}

#[test]
fn controller_picker_switches_atomically_and_preserves_prefab_overrides_undo_and_saved_protocol()
-> Result<()> {
    let mut f = Fixture::new()?;
    let animator = f.animator();
    let path = f.editor.project_root().join("assets/locomotion.ani");
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "version":1,"skeleton":animator.skeleton,"initial":"idle","parameters":{},
            "states":[{"name":"idle","looping":true,"motions":[{"threshold":0.,"clip":animator.clip.unwrap()}]}],"transitions":[]
        }))?,
    )?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let controller = comet_asset_pipeline::SourceMeta::load(&path)?.id;
    let instance = comet_core::InstanceId::new();
    let source_owner = f.selected;
    f.editor.document = Document::open_source(ContentSource {
        base: SceneSnapshot {
            version: comet_scene::scene::SCENE_VERSION,
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: f.prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = source_owner.in_instance(instance);
    let original = f.source();
    f.choose("单片段播放", "locomotion.ani");
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let changed = f.source();
    assert_eq!(f.animator().controller, Some(controller));
    assert_eq!(f.animator().clip, None);
    assert_eq!(changed.instances[0].overrides.len(), 2);
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    assert_eq!(f.source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.source(), changed);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), changed);
    f.editor.start(&mut f.assets)?;
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text.starts_with("当前状态：idle"))
    );
    assert_eq!(f.source(), changed);
    Ok(())
}

#[test]
fn authored_controls_keep_idle_assets_lazy_and_speed_transactions_cancel_undo_and_save()
-> Result<()> {
    let mut f = Fixture::new()?;
    f.evict();
    let original = f.source();
    for _ in 0..6 {
        f.frame(vec![]);
    }
    assert_eq!(
        f.assets.resident_count(),
        0,
        "collapsed bindings and clip menus use accepted metadata"
    );
    let clip = f.frame(vec![]);
    let row = label(&clip, "播放片段");
    let anchor = texts(&clip)
        .into_iter()
        .find(|(_, p)| (p.y - row.y).abs() < 2. && p.x > row.x + 20.)
        .unwrap()
        .1;
    f.click(anchor);
    f.frame(vec![escape()]);
    assert_eq!(
        f.assets.resident_count(),
        0,
        "opening clip choices must not load clip products"
    );
    assert!(Arc::ptr_eq(&original, &f.source()));
    f.press("循环");
    assert!(!f.animator().looping);
    let before_speed = f.source();
    let history = f.editor.document.history_len();
    let output = f.frame(vec![]);
    let start = speed(&output);
    let end = start + Vec2::new(30., 0.);
    f.frame(vec![Event::PointerMoved(start), button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    assert!(f.editor.document.is_editing());
    f.frame(vec![escape()]);
    f.frame(vec![button(end, false)]);
    assert_eq!(f.source(), before_speed);
    assert_eq!(f.editor.document.history_len(), history);
    let output = f.frame(vec![]);
    let start = speed(&output);
    let end = start + Vec2::new(40., 0.);
    f.frame(vec![Event::PointerMoved(start), button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    f.frame(vec![button(end, false)]);
    assert!(f.animator().speed > 1.);
    assert_eq!(f.editor.document.history_len(), history + 1);
    let changed = f.source();
    f.editor.document.undo()?;
    assert_eq!(f.source(), before_speed);
    f.editor.document.redo()?;
    assert_eq!(f.source(), changed);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), changed);
    f.press("移除动画");
    assert!(
        f.source()
            .base
            .entities
            .iter()
            .find(|entity| entity.id == f.selected)
            .unwrap()
            .animator
            .is_none()
    );
    assert!(
        f.source()
            .base
            .entities
            .iter()
            .filter_map(|entity| entity.skin.as_ref())
            .all(|skin| skin.animator.is_none())
    );
    f.editor.document.undo()?;
    assert_eq!(f.source(), changed);
    assert!(!f.editor.document.is_dirty());
    Ok(())
}

#[test]
fn editing_a_direct_prefab_binding_preserves_existing_child_refs_without_offering_nested_choices()
-> Result<()> {
    let mut f = Fixture::new()?;
    let mut parent = f.editor.document.source();
    let controller = f.selected;
    let animator = parent
        .base
        .entities
        .iter()
        .find(|entity| entity.id == controller)
        .unwrap()
        .animator
        .as_ref()
        .unwrap();
    let bindings = &animator.bindings;
    let skeleton_product = f.assets.load(animator.skeleton)?;
    let Product::Skeleton(skeleton) = &skeleton_product.payload else {
        unreachable!()
    };
    let nested_key = skeleton
        .nodes
        .iter()
        .find(|node| {
            node.parent.is_none()
                && !skeleton
                    .nodes
                    .iter()
                    .any(|child| child.parent == Some(node.id))
        })
        .unwrap()
        .id;
    let edited_node = skeleton
        .nodes
        .iter()
        .find(|node| node.parent.is_some())
        .unwrap();
    let edited_key = edited_node.id;
    let Some(EntityRef::Local {
        entity: replacement_parent,
    }) = bindings[&edited_node.parent.unwrap()]
    else {
        unreachable!()
    };
    let Some(EntityRef::Local { entity: nested_id }) = bindings[&nested_key] else {
        unreachable!()
    };
    let mut nested = parent
        .base
        .entities
        .iter()
        .find(|entity| entity.id == nested_id)
        .unwrap()
        .clone();
    nested.parent = None;
    nested.model = None;
    nested.skin = None;
    nested.animator = None;
    nested.links = Default::default();
    let child_path = f.editor.project_root().join("assets/child.prefab");
    fs::write(
        &child_path,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![nested],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    let child_asset = comet_asset_pipeline::SourceMeta::load(&child_path)?.id;
    let child = comet_core::InstanceId::new();
    parent.base.version = comet_scene::scene::SCENE_VERSION;
    parent.instances.push(PrefabInstance {
        translation: [0.; 3],
        id: child,
        prefab: child_asset,
        parent: Some(controller),
        overrides: vec![],
    });
    let mut replacement = EntityData::new("替换骨骼");
    replacement.parent = Some(replacement_parent);
    let replacement_id = replacement.id;
    parent.base.entities.push(replacement);
    let animator = parent
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == controller)
        .unwrap()
        .animator
        .as_mut()
        .unwrap();
    animator
        .bindings
        .insert(nested_key, Some(EntityRef::in_instance(child, nested_id)));
    animator.bindings.insert(edited_key, None);
    let parent_path = f.editor.project_root().join("assets/parent.prefab");
    fs::write(&parent_path, parent.to_json()?)?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let parent_asset = comet_asset_pipeline::SourceMeta::load(&parent_path)?.id;
    let outer = comet_core::InstanceId::new();
    f.editor.document = Document::open_source(ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: outer,
            prefab: parent_asset,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = controller.in_instance(outer);
    f.frame(vec![]);
    f.frame(vec![]);
    let original = f.source();
    let nested_world = nested_id.in_instance(child.in_instance(outer));
    let choices = f
        .editor
        .cached_field_references(&f.ctx, f.selected, &mut f.assets)?;
    assert!(!choices.targets.iter().any(|(id, _)| *id == nested_world));
    f.press("骨骼绑定");
    f.choose("未绑定", "替换骨骼");
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    let changed = f.source();
    let schema::FieldValue::AnimatorBindings(bindings) = &changed.instances[0].overrides[0].value
    else {
        unreachable!()
    };
    assert_eq!(
        bindings[&nested_key],
        Some(EntityRef::in_instance(child, nested_id))
    );
    assert_eq!(
        bindings[&edited_key],
        Some(EntityRef::local(replacement_id))
    );
    assert_eq!(f.editor.document.history_len(), 1);
    assert_eq!(
        f.animator().bindings[&nested_key],
        Some(EntityRef::local(nested_world))
    );
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), changed);
    f.editor.document.undo()?;
    assert_eq!(f.source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.source(), changed);

    // A subsequent edit reads the existing override as the preservation baseline.
    f.choose("替换骨骼", "未绑定");
    assert!(f.editor.error.is_none(), "{:?}", f.editor.error);
    assert_eq!(f.editor.document.history_len(), 2);
    let source = f.source();
    let schema::FieldValue::AnimatorBindings(bindings) = &source.instances[0].overrides[0].value
    else {
        unreachable!()
    };
    assert_eq!(
        bindings[&nested_key],
        Some(EntityRef::in_instance(child, nested_id))
    );
    assert_eq!(bindings[&edited_key], None);
    let choices = f
        .editor
        .cached_field_references(&f.ctx, f.selected, &mut f.assets)?;
    assert!(!choices.targets.iter().any(|(id, _)| *id == nested_world));
    Ok(())
}

#[test]
fn binding_repair_and_direct_prefab_fields_keep_source_ids_and_readonly_boundaries() -> Result<()> {
    let mut f = Fixture::new()?;
    let instance = comet_core::InstanceId::new();
    let source_controller = f.selected;
    f.editor.document = Document::open_source(ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: f.prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = source_controller.in_instance(instance);
    f.frame(vec![]);
    let mut animator = f.animator();
    let (&source, target) = animator.bindings.iter().next().unwrap();
    let target = target.clone().unwrap();
    let EntityRef::Local { entity: target_id } = target else {
        unreachable!()
    };
    animator.bindings.insert(source, None);
    f.editor
        .edit_animation(f.selected, Some(animator), None, &mut f.assets)?;
    f.editor.commit()?;
    let unbound = f.source();
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), unbound);
    f.press("骨骼绑定");
    let target_name = f
        .editor
        .last_snapshot
        .entities
        .iter()
        .find(|entity| entity.id == target_id)
        .unwrap()
        .name
        .clone();
    f.choose("未绑定", &target_name);
    let repaired = f.source();
    assert_ne!(repaired, unbound);
    assert_eq!(
        f.animator().bindings[&source],
        Some(EntityRef::local(target_id))
    );
    let override_value = repaired.instances[0]
        .overrides
        .iter()
        .find(|change| change.field == schema::ANIMATOR_BINDINGS)
        .unwrap();
    let schema::FieldValue::AnimatorBindings(bindings) = &override_value.value else {
        unreachable!()
    };
    let source_prefab = comet_scene::PrefabResolver::prefab(&mut f.assets, f.prefab)?;
    let source_target = source_prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.id.in_instance(instance) == target_id)
        .unwrap()
        .id;
    assert_eq!(bindings[&source], Some(EntityRef::local(source_target)));
    f.press("循环");
    assert!(
        f.source().instances[0]
            .overrides
            .iter()
            .any(|change| change.field == schema::ANIMATOR_LOOPING)
    );
    f.editor.document.undo()?;
    assert_eq!(f.source(), repaired);
    f.editor.document.redo()?;
    let before = f.source();
    f.editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    f.press("循环");
    assert!(Arc::ptr_eq(&before, &f.source()));
    f.editor.documents = Default::default();
    f.editor.refresh_resources()?;
    f.press("循环");
    assert!(Arc::ptr_eq(&before, &f.source()));
    Ok(())
}

#[test]
fn empty_authored_controller_creation_and_play_field_ownership_use_actual_widgets() -> Result<()> {
    let mut f = Fixture::new()?;
    let existing = f.source();
    let skeleton = existing
        .base
        .entities
        .iter()
        .find_map(|entity| entity.animator.as_ref())
        .unwrap()
        .skeleton;
    let blank = EntityData::new("新控制器");
    f.selected = blank.id;
    f.editor.document = Document::open(SceneSnapshot {
        entities: vec![blank],
        ..Default::default()
    })?;
    let output = f.frame(vec![]);
    let anchor = label(&output, "请选择骨架");
    f.click(anchor);
    let output = f.frame(vec![]);
    let option = texts(&output)
        .into_iter()
        .find(|(text, p)| text.contains("SimpleSkin.gltf") && p.y > anchor.y + 3.)
        .context("imported skeleton option")?
        .1;
    f.click(option);
    f.press("添加动画");
    assert_eq!(f.animator().skeleton, skeleton);
    assert!(f.animator().bindings.is_empty());
    f.press("移除动画");
    assert!(f.source().base.entities[0].animator.is_none());
    f.editor.document.undo()?;
    assert!(f.source().base.entities[0].animator.is_some());
    f.editor.document = Document::open_source((*existing).clone())?;
    f.selected = existing
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .unwrap()
        .id;
    f.editor.start(&mut f.assets)?;
    f.editor.play.as_mut().unwrap().set_paused(true);
    let state = f
        .editor
        .play
        .as_ref()
        .unwrap()
        .animation_state(f.selected)
        .unwrap();
    let before = f.source();
    f.press("循环");
    assert!(Arc::ptr_eq(&before, &f.source()));
    let output = f.frame(vec![]);
    assert!(texts(&output).iter().any(
        |(text, _)| text == &format!("片段时间：{:.3} / {:.3} 秒", state.time, state.duration)
    ));
    let snapshot = f.editor.play.as_mut().unwrap().shared_snapshot();
    let bone = snapshot
        .entities
        .iter()
        .find(|entity| {
            f.editor
                .play
                .as_ref()
                .unwrap()
                .is_animation_controlled(entity.id, schema::ROTATION)
        })
        .unwrap();
    f.selected = bone.id;
    f.full = true;
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "该字段由动画驱动，运行期间只读。")
    );
    let rotation = bone.transform.rotation;
    let row = label(&output, "旋转");
    let control = texts(&output)
        .into_iter()
        .find(|(text, p)| text.starts_with("X ") && p.y > row.y && p.y < row.y + 35.)
        .unwrap()
        .1;
    f.frame(vec![Event::PointerMoved(control), button(control, true)]);
    f.frame(vec![Event::PointerMoved(control + Vec2::new(40., 0.))]);
    f.frame(vec![button(control, false)]);
    assert_eq!(
        f.editor
            .play
            .as_mut()
            .unwrap()
            .shared_snapshot()
            .entities
            .iter()
            .find(|entity| entity.id == f.selected)
            .unwrap()
            .transform
            .rotation,
        rotation
    );
    assert!(
        !f.editor
            .play
            .as_ref()
            .unwrap()
            .is_animation_controlled(f.selected, schema::TRANSLATION)
    );
    Ok(())
}

#[test]
fn rebuilding_a_stale_binding_table_is_explicit_and_undo_restores_old_declarations() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut source = (*f.source()).clone();
    let animator = source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.selected)
        .unwrap()
        .animator
        .as_mut()
        .unwrap();
    let expected = animator.bindings.clone();
    let missing = *expected.keys().next().unwrap();
    animator.bindings.remove(&missing);
    let obsolete = EntityId::new();
    animator.bindings.insert(obsolete, None);
    f.editor.document = Document::open_source(source)?;
    let old = f.source();
    f.frame(vec![]);
    assert!(
        Arc::ptr_eq(&old, &f.source()),
        "opening the inspector must not prune stale source declarations"
    );
    f.press("骨骼绑定");
    f.press("重建绑定表");
    let rebuilt = f.animator();
    assert_eq!(
        rebuilt.bindings.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    assert_eq!(rebuilt.bindings[&missing], None);
    assert!(!rebuilt.bindings.contains_key(&obsolete));
    for (id, value) in expected {
        if id != missing {
            assert_eq!(rebuilt.bindings[&id], value);
        }
    }
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    assert_eq!(f.source(), old);
    Ok(())
}

#[test]
fn skin_controller_choices_keep_prefab_reference_scope_and_nested_instances_readonly() -> Result<()>
{
    let mut f = Fixture::new()?;
    let source = f.source();
    let controller = f.selected;
    let model = source
        .base
        .entities
        .iter()
        .find(|entity| entity.skin.is_some())
        .unwrap()
        .id;
    let controller_name = source
        .base
        .entities
        .iter()
        .find(|entity| entity.id == controller)
        .unwrap()
        .name
        .clone();
    let instance = comet_core::InstanceId::new();
    f.editor.document = Document::open_source(ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: f.prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = model.in_instance(instance);
    let before = f.source();
    f.choose(&controller_name, "未绑定");
    let unbound = f.source();
    assert_eq!(unbound.instances[0].overrides.len(), 1);
    assert_eq!(
        unbound.instances[0].overrides[0].field,
        schema::SKIN_ANIMATOR
    );
    assert_eq!(
        unbound.instances[0].overrides[0].value,
        schema::FieldValue::Entity(None)
    );
    f.editor.save(&mut f.assets)?;
    f.choose("未绑定", &controller_name);
    let repaired = f.source();
    assert_eq!(repaired.instances[0].overrides.len(), 1);
    assert_eq!(
        repaired.instances[0].overrides[0].value,
        schema::FieldValue::Entity(Some(EntityRef::local(controller)))
    );
    assert_eq!(
        f.editor
            .snapshot(&mut f.assets)?
            .entities
            .iter()
            .find(|entity| entity.id == f.selected)
            .unwrap()
            .skin
            .as_ref()
            .unwrap()
            .animator,
        Some(EntityRef::local(controller.in_instance(instance)))
    );
    f.editor.document.undo()?;
    assert_eq!(f.source(), unbound);
    f.editor.document.redo()?;
    assert_eq!(f.source(), repaired);
    let output = f.frame(vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text == "移除蒙皮绑定")
    );
    f.editor.document = Document::open_source((*before).clone())?;
    let nested_path = f.editor.project_root().join("assets/nested.prefab");
    fs::write(&nested_path, before.to_json()?)?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    f.editor.document = Document::open_source(ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: comet_core::InstanceId::new(),
            prefab: comet_asset_pipeline::SourceMeta::load(&nested_path)?.id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = f
        .editor
        .snapshot(&mut f.assets)?
        .entities
        .iter()
        .find(|entity| entity.skin.is_some())
        .unwrap()
        .id;
    let nested = f.source();
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "Prefab 组件结构或嵌套配置请在源 Prefab 中编辑。")
    );
    let anchor = label(&output, &controller_name);
    f.click(anchor);
    assert!(Arc::ptr_eq(&nested, &f.source()));
    assert!(
        !texts(&f.frame(vec![]))
            .iter()
            .any(|(text, _)| text == "未绑定")
    );
    Ok(())
}

#[test]
fn runtime_labels_distinguish_clip_pause_world_pause_actual_speed_and_completion() -> Result<()> {
    use comet_core::{Action, ActionEvent, ActionInput, Actions};
    let enabled = || ActionInput {
        enabled: true,
        ..Default::default()
    };
    let interact = || {
        let mut held = Actions::default();
        held.set(Action::Interact, true);
        ActionInput {
            enabled: true,
            held,
            events: vec![ActionEvent {
                action: Action::Interact,
                pressed: true,
            }],
            ..Default::default()
        }
    };
    let mut f = Fixture::new()?;
    let script = f
        .editor
        .project_root()
        .join("assets/ui-animation-state.rhai");
    fs::write(
        &script,
        r#"
        fn on_start(ctx) {
            this.phase = 0;
            ctx.set_animation_speed(ctx.entity(), 2.0);
        }
        fn on_fixed_update(ctx, dt) {
            if ctx.pressed("interact") {
                if this.phase == 0 { ctx.pause_animation(ctx.entity()); }
                else if this.phase == 1 { ctx.resume_animation(ctx.entity()); }
                else if this.phase == 2 { ctx.set_animation_speed(ctx.entity(), 0.0); }
                else { ctx.set_animation_speed(ctx.entity(), 2.0); }
                this.phase += 1;
            }
        }
    "#,
    )?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let script = comet_asset_pipeline::SourceMeta::load(&script)?.id;
    let mut source = (*f.source()).clone();
    let controller = source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.selected)
        .unwrap();
    controller.animator.as_mut().unwrap().looping = false;
    controller.script = Some(comet_scene::Script::new(script));
    f.editor.document = Document::open_source(source)?;
    f.editor.start(&mut f.assets)?;
    let author = f.source();
    f.editor.play.as_mut().unwrap().set_paused(true);
    let output = f.frame(vec![]);
    let labels: Vec<_> = texts(&output).into_iter().map(|(text, _)| text).collect();
    assert!(labels.iter().any(|text| text == "片段状态：播放中"));
    assert!(labels.iter().any(|text| text == "运行世界：已暂停"));
    assert!(labels.iter().any(|text| text == "实际速度：2.00×"));

    let play = f.editor.play.as_mut().unwrap();
    play.set_paused(false);
    play.advance_with_input(1. / 60., enabled())?;
    play.advance_with_input(1. / 60., interact())?;
    play.set_paused(true);
    let state = play.animation_state(f.selected).unwrap();
    assert!(!state.playing && state.speed == 2.);
    let output = f.frame(vec![]);
    let labels: Vec<_> = texts(&output).into_iter().map(|(text, _)| text).collect();
    assert!(labels.iter().any(|text| text == "片段状态：已暂停"));
    assert!(labels.iter().any(|text| text == "实际速度：2.00×"));
    assert!(labels.iter().any(|text| text == "运行世界：已暂停"));
    assert_eq!(
        f.animator().speed,
        1.,
        "runtime speed is separate from authored configuration"
    );

    let play = f.editor.play.as_mut().unwrap();
    play.set_paused(false);
    play.advance_with_input(1. / 60., enabled())?;
    play.advance_with_input(1. / 60., interact())?;
    play.advance_with_input(1. / 60., enabled())?;
    play.advance_with_input(1. / 60., interact())?;
    let frozen = play.animation_state(f.selected).unwrap();
    play.advance_with_input(1. / 60., enabled())?;
    assert_eq!(play.animation_state(f.selected).unwrap().time, frozen.time);
    let output = f.frame(vec![]);
    let labels: Vec<_> = texts(&output).into_iter().map(|(text, _)| text).collect();
    assert!(labels.iter().any(|text| text == "片段状态：播放中"));
    assert!(labels.iter().any(|text| text == "实际速度：0.00×"));
    assert!(!labels.iter().any(|text| text == "运行世界：已暂停"));

    let play = f.editor.play.as_mut().unwrap();
    play.advance_with_input(1. / 60., interact())?;
    for _ in 0..400 {
        play.advance_with_input(1. / 60., enabled())?;
    }
    let ended = play.animation_state(f.selected).unwrap();
    assert!(!ended.playing && ended.time == ended.duration);
    play.set_suspended(true);
    let output = f.frame(vec![]);
    let labels: Vec<_> = texts(&output).into_iter().map(|(text, _)| text).collect();
    assert!(labels.iter().any(|text| text == "片段状态：已结束"));
    assert!(labels.iter().any(|text| text == "运行世界：宿主挂起"));
    assert!(
        labels
            .iter()
            .any(|text| text == &format!("片段时间：{:.3} / {:.3} 秒", ended.time, ended.duration))
    );
    assert!(Arc::ptr_eq(&author, &f.source()));
    Ok(())
}
