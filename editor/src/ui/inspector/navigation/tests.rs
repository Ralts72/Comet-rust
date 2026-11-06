use super::*;
use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext};
use comet_core::{AssetId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{
    Character, ColliderShape, ContentSource, PhysicsBody, PrefabInstance, SceneSnapshot,
    schema::{self, FieldValue},
};
use egui::{Event, PointerButton, Pos2, Rect, Vec2};
use std::sync::Arc;
use std::{
    fs,
    time::{Duration, Instant},
};

struct Case {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    selected: EntityId,
    region: EntityId,
    npc: EntityId,
    goal: EntityId,
    floor: EntityId,
}
impl Case {
    fn new(region_present: bool, agent_present: bool) -> Result<Self> {
        let (temp, mut editor, assets) = crate::test_support::fixture()?;
        let mut region = EntityData::new("导航区域实体");
        region.navigation_region = region_present.then(|| NavigationRegion {
            half_extents: [4., 3., 4.],
            cell_size: 0.25,
            ..Default::default()
        });
        let mut floor = EntityData::new("静态地面");
        floor.transform.translation = [0., -0.25, 0.];
        floor.physics = Some(PhysicsBody {
            shape: ColliderShape::Box {
                half_extents: [3.5, 0.25, 3.5],
            },
            ..Default::default()
        });
        let mut npc = EntityData::new("导航角色");
        npc.transform.translation = [-2., 1., 0.];
        npc.physics = Some(PhysicsBody {
            motion: BodyMotion::Kinematic,
            shape: ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.3,
            },
            ..Default::default()
        });
        npc.character = Some(Character {
            player_controlled: false,
            ..Default::default()
        });
        let mut goal = EntityData::new("到达目标");
        goal.transform.translation = [2., 0., 0.];
        npc.navigation_agent = agent_present.then(|| NavigationAgent {
            target: Some(EntityRef::local(goal.id)),
            ..Default::default()
        });
        let ids = (region.id, npc.id, goal.id, floor.id);
        editor.document = Document::open(SceneSnapshot {
            entities: vec![region, floor, npc, goal],
            ..Default::default()
        })?;
        let ctx = egui::Context::default();
        comet_application::configure_ui(&ctx);
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.style_mut_of(theme, |style| style.animation_time = 0.);
        }
        Ok(Self {
            _temp: temp,
            editor,
            assets,
            ctx,
            selected: ids.0,
            region: ids.0,
            npc: ids.1,
            goal: ids.2,
            floor: ids.3,
        })
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        let snapshot = if self.editor.play.is_some() {
            Arc::new(self.editor.snapshot(&mut self.assets).unwrap())
        } else {
            self.editor.navigation_snapshot(&mut self.assets).unwrap()
        };
        self.editor.last_snapshot = snapshot.clone();
        let entity = snapshot
            .entities
            .iter()
            .find(|entity| entity.id == self.selected)
            .unwrap()
            .clone();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 900.))),
                events,
                ..Default::default()
            },
            |ui| {
                draw(&mut self.editor, ui, &mut self.assets, &entity);
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
    fn drag(&mut self, name: &str) {
        let output = self.frame(vec![]);
        let position = label(&output, name);
        let end = position + Vec2::new(40., 0.);
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![Event::PointerMoved(end)]);
        self.frame(vec![button(end, false)]);
    }
    fn choose(&mut self, current: &str, name: &str) {
        self.press(current);
        let output = self.frame(vec![]);
        self.click(label(&output, name));
    }
    fn wait(&mut self) -> Result<()> {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut number = 0;
        while self.editor.navigation.is_running() {
            let mut result = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))),
                    ..Default::default()
                },
                |ui| {
                    result = Some(self.editor.frame(
                        ui,
                        &mut self.assets,
                        &FrameContext {
                            number,
                            dt: 1. / 60.,
                            active: true,
                            activity_changed: false,
                            target_generation: 1,
                            size: [1280, 800],
                            smoke: false,
                        },
                    ));
                },
            );
            output.textures_delta.clear();
            result.context("host frame")??;
            number += 1;
            ensure!(Instant::now() < deadline, "navigation bake timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn find(shape: &egui::epaint::Shape, values: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => values.push((
                text.galley.job.text.clone(),
                text.pos + text.galley.rect.center().to_vec2(),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    find(shape, values);
                }
            }
            _ => {}
        }
    }
    let mut values = vec![];
    for shape in &output.shapes {
        find(&shape.shape, &mut values);
    }
    values
}
fn label(output: &egui::FullOutput, name: &str) -> Pos2 {
    texts(output)
        .into_iter()
        .find(|(text, _)| text == name || text.starts_with(&format!("{name} ")))
        .map(|(_, point)| point)
        .unwrap_or_else(|| panic!("missing {name}; {:?}", texts(output)))
}

#[test]
fn author_navigation_widgets_add_drag_cancel_target_and_remove_with_undo() -> Result<()> {
    let mut c = Case::new(false, false)?;
    c.press("添加导航区域");
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    assert_eq!(c.editor.document.history_len(), 1);
    let first = c.editor.document.shared_source();
    c.drag("网格尺寸");
    assert_eq!(c.editor.document.history_len(), 2);
    let changed = c.editor.document.shared_source();
    assert_ne!(first, changed);
    let output = c.frame(vec![]);
    let position = label(&output, "网格尺寸");
    let end = position + Vec2::new(40., 0.);
    c.frame(vec![Event::PointerMoved(position), button(position, true)]);
    c.frame(vec![Event::PointerMoved(end)]);
    assert!(c.editor.document.is_editing());
    c.frame(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    c.frame(vec![button(end, false)]);
    assert_eq!(c.editor.document.shared_source(), changed);
    assert_eq!(c.editor.document.history_len(), 2);
    c.press("移除导航区域");
    assert!(
        c.editor
            .snapshot(&mut c.assets)?
            .entities
            .iter()
            .find(|entity| entity.id == c.region)
            .unwrap()
            .navigation_region
            .is_none()
    );
    c.editor.document.undo()?;
    assert_eq!(c.editor.document.shared_source(), changed);
    c.selected = c.npc;
    c.press("添加导航代理");
    c.choose("无目标", "到达目标");
    let authored = c.editor.document.shared_source();
    assert_eq!(
        authored
            .base
            .entities
            .iter()
            .find(|entity| entity.id == c.npc)
            .unwrap()
            .navigation_agent
            .as_ref()
            .unwrap()
            .target,
        Some(EntityRef::local(c.goal))
    );
    c.press("移除导航代理");
    c.editor.document.undo()?;
    assert_eq!(c.editor.document.shared_source(), authored);
    c.editor.save(&mut c.assets)?;
    assert_eq!(Document::load(&c.editor.source)?.shared_source(), authored);
    Ok(())
}

#[test]
fn bake_widgets_accept_products_reject_stale_jobs_and_reuse_the_existing_navigation_identity()
-> Result<()> {
    let mut c = Case::new(true, true)?;
    let author_file = fs::read(&c.editor.source)?;
    c.press("烘焙导航");
    c.wait()?;
    assert!(
        c.editor.navigation.failure.is_none(),
        "{:?}",
        c.editor.navigation.failure
    );
    let source = c.editor.document.shared_source();
    let asset = source
        .base
        .entities
        .iter()
        .find(|entity| entity.id == c.region)
        .unwrap()
        .navigation_region
        .as_ref()
        .unwrap()
        .data
        .context("bake should bind navigation data")?;
    let path = c
        .editor
        .project_root()
        .join("assets")
        .join(format!("navigation-{}.nav", c.region));
    let old_bytes = fs::read(&path)?;
    let metadata = fs::read(comet_asset_pipeline::SourceMeta::path(&path))?;
    assert_eq!(
        fs::read(&c.editor.source)?,
        author_file,
        "baking does not silently save the author document"
    );
    assert_eq!(c.editor.document.history_len(), 1);
    c.editor.start(&mut c.assets)?;
    c.selected = c.npc;
    c.press("启用导航代理");
    c.drag("到达距离");
    assert!(Arc::ptr_eq(&source, &c.editor.document.shared_source()));
    c.editor.stop();
    c.selected = c.region;
    c.editor.edit(
        c.floor,
        schema::TRANSLATION,
        FieldValue::Vec3([0.25, -0.25, 0.]),
        &mut c.assets,
    )?;
    c.editor.commit()?;
    let output = c.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "导航数据已陈旧，请重新烘焙。")
    );
    let revision = c.assets.revision().to_owned();
    c.press("烘焙导航");
    c.editor.document.begin()?;
    c.editor
        .document
        .preview_name(c.region, "烘焙中发生的修改".into())?;
    let pending = c.editor.document.shared_source();
    c.wait()?;
    assert!(c.editor.navigation.failure.is_some());
    assert!(c.editor.document.is_editing());
    assert!(Arc::ptr_eq(&pending, &c.editor.document.shared_source()));
    assert_eq!(c.assets.revision(), revision);
    assert_eq!(
        fs::read(&path)?,
        old_bytes,
        "a stale pure bake must not replace an existing valid asset"
    );
    c.editor.cancel()?;
    c.press("烘焙导航");
    c.wait()?;
    assert!(
        c.editor.navigation.failure.is_none(),
        "{:?}",
        c.editor.navigation.failure
    );
    assert_eq!(comet_asset_pipeline::SourceMeta::load(&path)?.id, asset);
    assert_eq!(
        fs::read(comet_asset_pipeline::SourceMeta::path(&path))?,
        metadata
    );
    assert_ne!(fs::read(&path)?, old_bytes);
    Ok(())
}

#[test]
fn direct_prefab_target_widgets_store_source_uuids_and_nested_or_background_controls_stay_read_only()
-> Result<()> {
    let mut c = Case::new(true, false)?;
    let snapshot = c.editor.snapshot(&mut c.assets)?;
    let mut npc = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == c.npc)
        .unwrap()
        .clone();
    npc.name = "实例导航角色".into();
    npc.navigation_agent = Some(NavigationAgent::default());
    let mut goal = EntityData::new("实例目标");
    goal.transform.translation = [2., 0., 0.];
    let source_npc = npc.id;
    let source_goal = goal.id;
    let prefab_path = c.editor.project_root().join("assets/navigator.prefab");
    fs::write(
        &prefab_path,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![npc, goal],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    c.assets.refresh()?;
    let prefab = comet_asset_pipeline::SourceMeta::load(&prefab_path)?.id;
    let instance = InstanceId::new();
    let authoring: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|entity| entity.id == c.region || entity.id == c.floor)
        .cloned()
        .collect();
    let make_document = |prefab: AssetId, instance| ContentSource {
        base: SceneSnapshot {
            entities: authoring.clone(),
            ..Default::default()
        },
        instances: vec![PrefabInstance {
            id: instance,
            prefab,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    };
    c.editor.document = Document::open_source(make_document(prefab, instance))?;
    c.selected = source_npc.in_instance(instance);
    c.choose("无目标", "实例目标");
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let current = c.editor.document.shared_source();
    assert_eq!(current.instances[0].overrides.len(), 1);
    assert_eq!(
        current.instances[0].overrides[0].value,
        FieldValue::Entity(Some(EntityRef::local(source_goal)))
    );
    assert_eq!(
        c.editor
            .snapshot(&mut c.assets)?
            .entities
            .iter()
            .find(|entity| entity.id == c.selected)
            .unwrap()
            .navigation_agent
            .as_ref()
            .unwrap()
            .target,
        Some(EntityRef::local(source_goal.in_instance(instance)))
    );
    c.editor.save(&mut c.assets)?;
    assert_eq!(Document::load(&c.editor.source)?.shared_source(), current);
    c.frame(vec![]);
    c.assets.evict(prefab);
    let resident = c.assets.resident_count();
    for _ in 0..20 {
        c.frame(vec![]);
    }
    assert_eq!(
        c.assets.resident_count(),
        resident,
        "idle target choices must reuse the accepted snapshot and one small cache"
    );

    let paths = comet_core::ProjectPaths::open(c.editor.project_root())?;
    c.editor.resource_build.start(
        &paths,
        &c.editor.products,
        c.editor.document.world_id(),
        c.editor.document.revision(),
        c.editor.document.retained_sources().collect(),
        c.editor.document.prefab_asset(),
    )?;
    c.press("启用导航代理");
    c.drag("到达距离");
    assert!(Arc::ptr_eq(&current, &c.editor.document.shared_source()));
    let deadline = Instant::now() + Duration::from_secs(20);
    while c.editor.resource_build.is_running() {
        if let Some(result) = c.editor.resource_build.poll() {
            result.map_err(anyhow::Error::msg)?;
        }
        ensure!(Instant::now() < deadline, "resource preparation timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    c.editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    c.press("启用导航代理");
    c.drag("到达距离");
    assert!(Arc::ptr_eq(&current, &c.editor.document.shared_source()));
    c.editor.documents = Default::default();

    let inner = InstanceId::new();
    let outer = InstanceId::new();
    let wrapper_path = c.editor.project_root().join("assets/wrapper.prefab");
    fs::write(
        &wrapper_path,
        ContentSource {
            instances: vec![PrefabInstance {
                id: inner,
                prefab,
                parent: None,
                overrides: vec![],
            }],
            ..Default::default()
        }
        .to_json()?,
    )?;
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    c.assets.refresh()?;
    let wrapper = comet_asset_pipeline::SourceMeta::load(&wrapper_path)?.id;
    c.editor.document = Document::open_source(make_document(wrapper, outer))?;
    c.selected = source_npc.in_instance(inner.in_instance(outer));
    let source = c.editor.document.shared_source();
    c.press("启用导航代理");
    c.drag("到达距离");
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    assert!(Arc::ptr_eq(&source, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), 0);
    Ok(())
}
