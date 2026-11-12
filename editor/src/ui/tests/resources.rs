use super::*;
use comet_core::{EntityId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{BodyMotion, ContentSource, PhysicsBody, PrefabInstance, SceneSnapshot};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

mod browser;

struct Case {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    number: u64,
    source_roots: [EntityId; 2],
    nested: Option<InstanceId>,
    resource: &'static str,
    sources: Vec<(PathBuf, Vec<u8>)>,
}

fn data_case() -> Result<(Case, comet_core::AssetId)> {
    let mut c = Case::new(false)?;
    let path = c.editor.project_root().join("assets").join("stats.csv");
    fs::write(
        &path,
        "id,hp,ready,note,title\nhero,7,true,\"{\"\"drops\"\":[1,2]}\",\"英雄, \"\"甲\"\"\"\nguard,9,false,[],守卫\n",
    )?;
    fs::write(c.editor.project_root().join("assets").join("stats.csv.schema.json"), br#"{"fields":{"hp":{"type":"integer","min":1},"ready":{"type":"bool"},"note":{"type":"json"},"title":{"type":"string"}}}"#)?;
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    c.assets.refresh()?;
    c.editor.sources = comet_asset_pipeline::SourceCatalog::read(
        &comet_core::ProjectPaths::open(c.editor.project_root())?,
        c.assets.index(),
    )?;
    let id = comet_asset_pipeline::SourceMeta::load(&path)?.id;
    Ok((c, id))
}

#[test]
fn data_preview_keeps_authoring_private_and_follows_atomic_refresh() -> Result<()> {
    let (mut c, id) = data_case()?;
    let authoring = c.editor.document.shared_source();
    let history = c.editor.document.history_len();
    c.frame(vec![]);
    c.press("stats.csv", Rect::EVERYTHING);
    for _ in 0..3 {
        c.frame(vec![]);
    }
    assert_eq!(c.editor.resource_selection, Some(id));
    let output = c.frame(vec![]);
    for label in ["记录：2  字段：4", "hero", "7", "true", "{\"drops\":[1,2]}"] {
        position(&output, label, Rect::EVERYTHING);
    }
    let schema = c
        .editor
        .project_root()
        .join("assets")
        .join("stats.csv.schema.json");
    let accepted = fs::read(&schema)?;
    let revision = c.assets.revision().to_owned();
    fs::write(&schema, b"broken")?;
    c.editor.refresh_resources()?;
    c.wait_resources()?;
    assert!(c.editor.resource_build.failure.is_some());
    assert_eq!(c.assets.revision(), revision);
    assert_eq!(c.editor.resource_selection, Some(id));
    position(&c.frame(vec![]), "7", Rect::EVERYTHING);
    fs::write(&schema, &accepted)?;
    let path = c.editor.project_root().join("assets").join("stats.csv");
    let bytes = fs::read_to_string(&path)?.replace("hero,7,", "hero,11,");
    fs::write(&path, &bytes)?;
    c.editor.refresh_resources()?;
    c.wait_resources()?;
    assert!(
        c.editor.resource_build.failure.is_none(),
        "{:?}",
        c.editor.error
    );
    assert_eq!(c.editor.resource_selection, None);
    c.press("stats.csv", Rect::EVERYTHING);
    for _ in 0..3 {
        c.frame(vec![]);
    }
    position(&c.frame(vec![]), "11", Rect::EVERYTHING);
    c.editor.start(&mut c.assets)?;
    assert!(c.editor.refresh_resources().is_err());
    c.frame(vec![]);
    c.editor.stop();
    assert!(Arc::ptr_eq(&authoring, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), history);
    assert_eq!(fs::read_to_string(path)?, bytes);
    assert_eq!(fs::read(schema)?, accepted);
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_data_preview_draws_typed_records_without_editing_authoring() -> Result<()> {
    let (mut c, id) = data_case()?;
    c.frame(vec![]);
    c.editor.resource_selection = Some(id);
    for _ in 0..3 {
        c.frame(vec![]);
    }
    position(&c.frame(vec![]), "记录：2  字段：4", Rect::EVERYTHING);
    let path = c.editor.project_root().join("assets").join("stats.csv");
    let bytes = fs::read(&path)?;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    let mut options = comet_application::HostOptions::new("Comet · 数据");
    options.headless = true;
    options.smoke = true;
    options.report = Some(out.join("editor-data-preview.json"));
    options.capture = Some(out.join("editor-data-preview.png"));
    comet_application::run(options, c.assets, c.editor)?;
    assert_eq!(fs::read(path)?, bytes);
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("editor-data-preview.json"))?)?;
    assert_eq!(report["application"]["dirty"], false);
    assert_eq!(report["application"]["history"], 0);
    assert_eq!(report["application"]["play_active"], false);
    assert!(report["application"]["error"].is_null());
    assert_eq!(report["submitted_frames"], 8);
    Ok(())
}

impl Case {
    fn wait_resources(&mut self) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.editor.resource_build.is_running() {
            self.frame(vec![]);
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "resource refresh timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(())
    }
    fn new(nested: bool) -> Result<Self> {
        let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
        let model = editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .find(|entity| entity.name == "Amber cube")
            .unwrap()
            .model
            .clone();
        let bodies = ["落体甲", "落体乙"].map(|name| {
            let mut entity = EntityData::new(name);
            entity.model = model.clone();
            entity.physics = Some(PhysicsBody {
                motion: BodyMotion::Dynamic,
                ..Default::default()
            });
            entity.transform.translation = if name == "落体甲" {
                [-1.5, 5., 0.]
            } else {
                [1.5, 5., 0.]
            };
            entity
        });
        let source_roots = bodies.each_ref().map(|entity| entity.id);
        let directory = editor.project_root().join("assets");
        let direct = directory.join("rigid.prefab");
        fs::write(
            &direct,
            ContentSource {
                base: SceneSnapshot {
                    entities: bodies.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_json()?,
        )?;
        comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
        let inner = nested.then(InstanceId::new);
        let mut paths = vec![direct.clone()];
        if let Some(id) = inner {
            let wrapper = directory.join("nested.prefab");
            fs::write(
                &wrapper,
                ContentSource {
                    instances: vec![PrefabInstance {
                        translation: [0.; 3],
                        id,
                        prefab: comet_asset_pipeline::SourceMeta::load(&direct)?.id,
                        parent: None,
                        overrides: vec![],
                    }],
                    ..Default::default()
                }
                .to_json()?,
            )?;
            paths.push(wrapper);
            comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
        }
        assets.refresh()?;
        editor.sources = comet_asset_pipeline::SourceCatalog::read(
            &comet_core::ProjectPaths::open(editor.project_root())?,
            assets.index(),
        )?;
        editor.document = Document::open(Default::default())?;
        let mut sources = vec![];
        for path in paths {
            sources.push((path.clone(), fs::read(&path)?));
            let metadata = comet_asset_pipeline::SourceMeta::path(&path);
            sources.push((metadata.clone(), fs::read(metadata)?));
        }
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
            number: 0,
            source_roots,
            nested: inner,
            resource: if nested {
                "nested.prefab"
            } else {
                "rigid.prefab"
            },
            sources,
        })
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.frame_result(events).1
    }

    fn frame_result(&mut self, events: Vec<egui::Event>) -> (ViewRequest, egui::FullOutput) {
        let number = self.number;
        self.number += 1;
        frame_with_output(
            &self.ctx,
            &mut self.editor,
            &mut self.assets,
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

    fn click(&mut self, position: Pos2, modifiers: egui::Modifiers) -> egui::FullOutput {
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        self.frame(vec![egui::Event::PointerMoved(position), button(true)]);
        self.frame(vec![button(false)])
    }

    fn context_menu(&mut self, label: &str) -> egui::FullOutput {
        let output = self.frame(vec![]);
        let pos = position(&output, label, hierarchy());
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        self.frame(vec![])
    }

    fn press(&mut self, label: &str, region: Rect) -> egui::FullOutput {
        let output = self.frame(vec![]);
        self.click(position(&output, label, region), egui::Modifiers::NONE)
    }

    fn drag(&mut self, position: Pos2) {
        let end = position + Vec2::new(40., 0.);
        self.frame(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        self.frame(vec![egui::Event::PointerMoved(end)]);
        self.frame(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        assert!(self.editor.error.is_none(), "{:?}", self.editor.error);
    }

    fn roots(&self, outer: InstanceId) -> [EntityId; 2] {
        let scope = self.nested.map_or(outer, |inner| inner.in_instance(outer));
        self.source_roots.map(|id| id.in_instance(scope))
    }
}

fn hierarchy() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(220., 650.))
}
fn resources() -> Rect {
    Rect::from_min_max(Pos2::new(220., 660.), Pos2::new(1000., 775.))
}
fn inspector() -> Rect {
    Rect::from_min_max(Pos2::new(1000., 45.), Pos2::new(1280., 650.))
}
fn position(output: &egui::FullOutput, label: &str, region: Rect) -> Pos2 {
    fn find(shape: &egui::epaint::Shape, label: &str, region: Rect, exact: bool) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text)
                if text.galley.job.text == label
                    || (!exact
                        && (text.galley.job.text.ends_with(label)
                            || text.galley.job.text.starts_with(&format!("{label} ")))) =>
            {
                let point = text.pos + text.galley.rect.center().to_vec2();
                region.contains(point).then_some(point)
            }
            egui::epaint::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| find(shape, label, region, exact)),
            _ => None,
        }
    }
    // A window title can share a suffix with its action button.
    [true, false]
        .into_iter()
        .find_map(|exact| {
            output
                .shapes
                .iter()
                .find_map(|shape| find(&shape.shape, label, region, exact))
        })
        .unwrap_or_else(|| panic!("missing {label} in {region:?}"))
}

fn exercise_physics_resource(nested: bool) -> Result<()> {
    let mut c = Case::new(nested)?;
    c.frame(vec![]);
    c.press(c.resource, resources());
    c.press("添加到场景", resources());
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let placed = c.editor.document.shared_source();
    assert!(
        placed.base.entities.is_empty(),
        "physical roots do not acquire an author container"
    );
    assert_eq!(placed.instances.len(), 1);
    let instance = placed.instances[0].id;
    assert_eq!(placed.instances[0].parent, None);
    let roots = c.roots(instance);
    assert_eq!(
        c.editor
            .selection
            .ids()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        roots.into_iter().collect()
    );
    assert_eq!(c.editor.document.history_len(), 1);
    let snapshot = c.editor.snapshot(&mut c.assets)?;
    assert_eq!(snapshot.entities.len(), 2);
    assert!(
        snapshot
            .entities
            .iter()
            .all(|entity| entity.parent.is_none() && entity.physics.is_some())
    );

    c.press("落体甲", hierarchy());
    let output = c.frame(vec![]);
    let mass = position(&output, "质量 (kg)", inspector());
    c.drag(mass);
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let authored = c.editor.document.shared_source();
    if nested {
        assert!(
            Arc::ptr_eq(&authored, &placed),
            "nested configuration is read-only"
        );
        assert_eq!(c.editor.document.history_len(), 1);
    } else {
        assert_eq!(c.editor.document.history_len(), 2);
        assert_eq!(authored.instances[0].overrides.len(), 1);
        assert_eq!(authored.instances[0].overrides[0].entity, c.source_roots[0]);
        assert!(
            c.editor
                .snapshot(&mut c.assets)?
                .entities
                .iter()
                .find(|entity| entity.id == roots[0])
                .unwrap()
                .physics
                .as_ref()
                .unwrap()
                .mass
                > 1.
        );
    }

    let before_play = c.editor.snapshot(&mut c.assets)?;
    c.press("运行", Rect::EVERYTHING);
    assert!(c.editor.play.is_some(), "{:?}", c.editor.error);
    assert_ne!(c.editor.world(), c.editor.document.world_id());
    for _ in 0..6 {
        c.frame(vec![]);
    }
    let running = c.editor.snapshot(&mut c.assets)?;
    assert!(
        running
            .entities
            .iter()
            .all(|entity| entity.transform.translation[1] < 5.)
    );
    c.press("落体甲", hierarchy());
    let output = c.frame(vec![]);
    c.drag(position(&output, "质量 (kg)", inspector()));
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    assert!(Arc::ptr_eq(&authored, &c.editor.document.shared_source()));
    assert_eq!(
        c.editor
            .snapshot(&mut c.assets)?
            .entities
            .iter()
            .find(|entity| entity.id == roots[0])
            .unwrap()
            .physics,
        before_play
            .entities
            .iter()
            .find(|entity| entity.id == roots[0])
            .unwrap()
            .physics,
        "disabled Play controls must not change runtime body configuration"
    );
    c.press("停止", Rect::EVERYTHING);
    assert_eq!(c.editor.snapshot(&mut c.assets)?, before_play);

    c.press("落体甲", hierarchy());
    let output = c.frame(vec![]);
    c.click(
        position(&output, "落体乙", hierarchy()),
        egui::Modifiers::COMMAND,
    );
    assert_eq!(c.editor.selection.len(), 2);
    assert_eq!(c.editor.selection.active(), Some(roots[1]));
    let history = c.editor.document.history_len();
    c.key(egui::Key::D, egui::Modifiers::COMMAND);
    let copied = c.editor.document.shared_source();
    assert_eq!(
        copied.instances.len(),
        2,
        "selected roots from the same instance copy it once"
    );
    assert!(copied.base.entities.is_empty());
    let duplicate = copied
        .instances
        .iter()
        .find(|copy| copy.id != instance)
        .unwrap()
        .id;
    assert_eq!(c.editor.selection.active(), Some(c.roots(duplicate)[1]));
    assert_eq!(c.editor.selection.len(), 2);
    assert_eq!(c.editor.document.history_len(), history + 1);
    assert_eq!(c.editor.snapshot(&mut c.assets)?.entities.len(), 4);

    let selected_copy = c.editor.selection.ids();
    c.editor
        .selection
        .replace([roots[0], c.roots(duplicate)[0]]);
    assert!(c.editor.detach_resource(roots[0], &mut c.assets).is_err());
    assert_eq!(c.editor.document.shared_source(), copied);
    assert_eq!(c.editor.document.history_len(), history + 1);
    c.editor.selection.replace(selected_copy);
    c.key(egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(c.editor.document.shared_source(), authored);
    assert!(c.editor.selection.is_empty());
    assert_eq!(c.editor.document.history_len(), history + 2);
    c.key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(c.editor.document.shared_source(), copied);
    c.key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(c.editor.document.shared_source(), authored);
    c.key(
        egui::Key::Z,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    assert_eq!(c.editor.document.shared_source(), copied);
    c.key(
        egui::Key::Z,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    assert_eq!(c.editor.document.shared_source(), authored);
    c.key(egui::Key::S, egui::Modifiers::COMMAND);
    assert!(!c.editor.document.is_dirty());
    let source = c.editor.source.clone();
    c.editor.switch_document(source, &mut c.assets)?;
    assert_eq!(c.editor.document.shared_source(), authored);
    assert!(
        c.editor
            .snapshot(&mut c.assets)?
            .entities
            .iter()
            .all(|entity| roots.contains(&entity.id)
                && entity.parent.is_none()
                && entity.physics.is_some())
    );
    // Both physical roots belong to one declaration. Detach one level per click,
    // including a wrapper containing only nested instances.
    c.editor.selection.replace(roots);
    let batch_before = c.editor.document.shared_source();
    let batch_world = comet_scene::world_transforms(&c.editor.snapshot(&mut c.assets)?)?;
    let batch_history = c.editor.document.history_len();
    c.context_menu("落体乙");
    c.press("批量放置...", Rect::EVERYTHING);
    c.press("放置", Rect::EVERYTHING);
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let batch = c.editor.document.shared_source();
    assert!(batch.base.entities.is_empty());
    assert_eq!(batch.instances.len(), 4);
    assert_eq!(c.editor.document.history_len(), batch_history + 1);
    assert_eq!(c.editor.selection.len(), 6);
    let active = c.editor.selection.active().unwrap();
    let snapshot = c.editor.snapshot(&mut c.assets)?;
    let placed = comet_scene::world_transforms(&snapshot)?;
    let mut offsets = Vec::new();
    for copy in batch.instances.iter().filter(|i| i.id != instance) {
        let copied_roots = c.roots(copy.id);
        offsets.push(copy.translation[0]);
        for (old, new) in roots.into_iter().zip(copied_roots) {
            let actual = placed[&new].w_axis.truncate() - batch_world[&old].w_axis.truncate();
            assert!(actual.abs_diff_eq(
                comet_core::math::Vec3::new(copy.translation[0], 0., 0.),
                1e-5
            ));
            assert!(c.editor.selection.contains(new));
        }
        if copy.translation[0] == 6. {
            assert_eq!(active, copied_roots[1]);
        }
    }
    offsets.sort_by(f32::total_cmp);
    assert_eq!(offsets, [2., 4., 6.]);
    c.key(egui::Key::S, egui::Modifiers::COMMAND);
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    let mut standalone = AssetStore::open(&c.editor.products)?;
    let identity = comet_asset_pipeline::SourceMeta::load(&c.editor.source)?.id;
    assert_eq!(standalone.expand_scene(identity)?, snapshot);
    c.key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert!(Arc::ptr_eq(
        &batch_before,
        &c.editor.document.shared_source()
    ));
    c.key(
        egui::Key::Z,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    assert!(Arc::ptr_eq(&batch, &c.editor.document.shared_source()));
    c.key(egui::Key::Z, egui::Modifiers::COMMAND);
    c.key(egui::Key::S, egui::Modifiers::COMMAND);
    c.editor.selection.replace(roots);
    let before_detach = c.editor.snapshot(&mut c.assets)?;
    let depth = if nested { 2 } else { 1 };
    for level in 0..depth {
        let before = c.editor.document.shared_source();
        let history = c.editor.document.history_len();
        let output = c.context_menu("落体乙");
        let detach = position(&output, "解除 Prefab 关联", Rect::EVERYTHING);
        c.click(detach, egui::Modifiers::NONE);
        assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
        assert_eq!(c.editor.snapshot(&mut c.assets)?, before_detach);
        assert_eq!(c.editor.document.history_len(), history + 1);
        assert_eq!(c.editor.selection.len(), 2);
        assert_eq!(c.editor.selection.active(), Some(roots[1]));
        let after = c.editor.document.shared_source();
        if nested && level == 0 {
            assert!(after.base.entities.is_empty());
            assert_eq!(after.instances.len(), 1);
            assert_eq!(
                after.instances[0].id,
                c.nested.unwrap().in_instance(instance)
            );
        } else {
            assert!(after.instances.is_empty());
            assert_eq!(after.base.entities.len(), 2);
        }
        c.key(egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(c.editor.document.shared_source(), before);
        c.key(
            egui::Key::Z,
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        );
        assert_eq!(c.editor.document.shared_source(), after);
    }
    c.key(egui::Key::S, egui::Modifiers::COMMAND);
    c.editor
        .switch_document(c.editor.source.clone(), &mut c.assets)?;
    assert_eq!(c.editor.snapshot(&mut c.assets)?, before_detach);
    assert!(c.editor.document.shared_source().instances.is_empty());
    for (path, bytes) in &c.sources {
        assert_eq!(
            &fs::read(path)?,
            bytes,
            "placing and editing instances must not write {}",
            path.display()
        );
    }
    Ok(())
}

#[test]
fn multi_root_physics_resource_uses_real_roots_for_properties_play_and_atomic_structure_commands()
-> Result<()> {
    exercise_physics_resource(false)
}

#[test]
fn pure_nested_physics_resource_places_and_copies_the_outer_instance_with_its_primary_root()
-> Result<()> {
    exercise_physics_resource(true)
}

#[test]
fn model_detach_rejects_author_containers_before_committing_and_preserves_expanded_content()
-> Result<()> {
    let mut c = Case::new(false)?;
    let choice = c
        .editor
        .sources
        .of_kind(comet_asset_data::product::AssetKind::Prefab)
        .find(|source| source.source == std::path::Path::new("showcase.gltf"))
        .unwrap()
        .clone();
    c.editor
        .place_resource(choice.id, "模型容器", [0.; 3], &mut c.assets)?;
    let container = c.editor.selection.active().unwrap();
    c.editor.rename_object(container, "名称预览".into())?;
    let accepted = c.editor.document.shared_source();
    let history = c.editor.document.history_len();
    assert!(c.editor.detach_resource(container, &mut c.assets).is_err());
    assert!(Arc::ptr_eq(&accepted, &c.editor.document.shared_source()));
    assert!(c.editor.document.is_editing());
    assert_eq!(c.editor.document.history_len(), history);
    c.editor.cancel()?;
    let snapshot = c.editor.snapshot(&mut c.assets)?;
    let root = snapshot
        .entities
        .iter()
        .find(|entity| entity.parent == Some(container))
        .unwrap();
    let root_id = root.id;
    let name = root.name.clone();
    c.frame(vec![]);
    let output = c.context_menu(&name);
    let detach = position(&output, "解除模型关联", Rect::EVERYTHING);
    c.click(detach, egui::Modifiers::NONE);
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    assert_eq!(c.editor.snapshot(&mut c.assets)?, snapshot);
    assert!(c.editor.document.shared_source().instances.is_empty());
    assert_eq!(c.editor.selection.active(), Some(root_id));
    c.editor.document.undo()?;
    assert_eq!(c.editor.document.shared_source().instances.len(), 1);
    c.editor.start(&mut c.assets)?;
    assert!(c.editor.detach_resource(root_id, &mut c.assets).is_err());
    Ok(())
}

#[test]
fn placement_window_cancels_without_editing_and_places_model_groups_in_one_undo_step() -> Result<()>
{
    let mut c = Case::new(false)?;
    let choice = c
        .editor
        .sources
        .of_kind(comet_asset_data::product::AssetKind::Prefab)
        .find(|source| source.source == std::path::Path::new("showcase.gltf"))
        .unwrap()
        .clone();
    c.editor
        .place_resource(choice.id, "放置模型", [0.; 3], &mut c.assets)?;
    let root = c.editor.selection.active().unwrap();
    c.editor.rename_object(root, "字段预览".into())?;
    let preview = c.editor.document.shared_source();
    assert!(
        c.editor
            .place_copies(root, 3, [2., 0., 0.], &mut c.assets)
            .is_err()
    );
    assert!(Arc::ptr_eq(&preview, &c.editor.document.shared_source()));
    assert!(c.editor.document.is_editing());
    c.editor.cancel()?;
    let before = c.editor.document.shared_source();
    let history = c.editor.document.history_len();
    c.frame(vec![]);
    c.context_menu("放置模型");
    c.press("批量放置...", Rect::EVERYTHING);
    let output = c.frame(vec![]);
    position(&output, "新增份数", Rect::EVERYTHING);
    position(&output, "每份间距 XYZ", Rect::EVERYTHING);
    c.press("取消", Rect::EVERYTHING);
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), history);

    c.context_menu("放置模型");
    c.press("批量放置...", Rect::EVERYTHING);
    c.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    c.context_menu("放置模型");
    c.press("批量放置...", Rect::EVERYTHING);
    c.editor.selection.replace([root]);
    let output = c.frame(vec![]);
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.job.text == "每份间距 XYZ")));
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));

    c.context_menu("放置模型");
    c.press("批量放置...", Rect::EVERYTHING);
    let (_, output) = frame_with_output(
        &c.ctx,
        &mut c.editor,
        &mut c.assets,
        FrameContext {
            number: c.number,
            dt: 1. / 60.,
            active: true,
            activity_changed: true,
            target_generation: 1,
            size: [1280, 800],
            smoke: false,
        },
        vec![],
    );
    c.number += 1;
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.job.text == "每份间距 XYZ")));
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));

    c.context_menu("放置模型");
    c.press("批量放置...", Rect::EVERYTHING);
    c.press("放置", Rect::EVERYTHING);
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let after = c.editor.document.shared_source();
    assert_eq!(after.instances.len(), 4);
    assert_eq!(after.base.entities.len(), 4);
    assert_eq!(c.editor.document.history_len(), history + 1);
    assert_eq!(c.editor.selection.len(), 3);
    assert!(!c.editor.selection.contains(root));
    let active = c.editor.selection.active().unwrap();
    assert_eq!(
        after
            .base
            .entities
            .iter()
            .find(|entity| entity.id == active)
            .unwrap()
            .transform
            .translation,
        [6., 0., 0.]
    );
    let mut positions: Vec<_> = after
        .base
        .entities
        .iter()
        .filter(|entity| entity.id != root)
        .map(|entity| entity.transform.translation[0])
        .collect();
    positions.sort_by(f32::total_cmp);
    assert_eq!(positions, [2., 4., 6.]);
    for instance in &after.instances {
        assert_eq!(instance.prefab, choice.id);
        assert!(
            after
                .base
                .entities
                .iter()
                .any(|entity| Some(entity.id) == instance.parent)
        );
    }
    c.key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(c.editor.document.shared_source(), before);
    c.key(
        egui::Key::Z,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    assert_eq!(c.editor.document.shared_source(), after);
    c.key(egui::Key::S, egui::Modifiers::COMMAND);
    comet_asset_pipeline::prepare(c.editor.project_root(), &c.editor.products)?;
    let mut products = AssetStore::open(&c.editor.products)?;
    let meta = comet_asset_pipeline::SourceMeta::load(&c.editor.source)?;
    assert_eq!(
        products.expand_scene(meta.id)?,
        c.editor.snapshot(&mut c.assets)?
    );
    c.editor.start(&mut c.assets)?;
    assert!(
        c.editor
            .place_copies(root, 3, [2., 0., 0.], &mut c.assets)
            .is_err()
    );
    assert_eq!(c.editor.document.shared_source(), after);
    Ok(())
}

#[test]
fn save_prefab_browser_retries_without_touching_the_document_and_rebuilt_products_place_independently()
-> Result<()> {
    let mut c = Case::new(false)?;
    let choice = c
        .editor
        .sources
        .of_kind(comet_asset_data::product::AssetKind::Prefab)
        .find(|source| source.source == std::path::Path::new("showcase.gltf"))
        .unwrap()
        .clone();
    c.editor
        .place_resource(choice.id, "保存模型", [0.; 3], &mut c.assets)?;
    let root = c.editor.selection.active().unwrap();
    let before = c.editor.document.shared_source();
    let state = (
        c.editor.document.world_id(),
        c.editor.document.revision(),
        c.editor.document.state_id(),
        c.editor.document.history_len(),
        c.editor.document.is_dirty(),
    );
    let scene_path = c.editor.source.clone();
    let scene_bytes = fs::read(&scene_path)?;
    let model_meta = comet_asset_pipeline::SourceMeta::path(
        &c.editor.project_root().join("assets/showcase.gltf"),
    );
    let model_meta_bytes = fs::read(&model_meta)?;
    let set_path = |c: &mut Case, value: &str| {
        let output = c.frame(vec![]);
        let field = position(&output, "new.prefab", Rect::EVERYTHING);
        c.click(field, egui::Modifiers::NONE);
        c.key(egui::Key::A, egui::Modifiers::COMMAND);
        c.frame(vec![egui::Event::Text(value.into())]);
    };
    c.frame(vec![]);
    c.context_menu("保存模型");
    c.press("保存为 Prefab...", Rect::EVERYTHING);
    assert!(c.editor.documents.is_blocking() && !c.editor.documents.pending);
    c.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!c.editor.documents.is_blocking());
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    assert!(!c.editor.project_root().join("assets/new.prefab").exists());

    c.context_menu("保存模型");
    c.press("保存为 Prefab...", Rect::EVERYTHING);
    set_path(&mut c, "../escape.prefab");
    c.press("保存", Rect::EVERYTHING);
    assert!(c.editor.documents.is_blocking());
    assert!(!c.editor.project_root().join("escape.prefab").exists());
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    // Keep the entered path and retry in the same browser.
    let output = c.frame(vec![]);
    c.click(
        position(&output, "../escape.prefab", Rect::EVERYTHING),
        egui::Modifiers::NONE,
    );
    c.key(egui::Key::A, egui::Modifiers::COMMAND);
    c.frame(vec![egui::Event::Text("saved.prefab".into())]);
    c.press("保存", Rect::EVERYTHING);
    assert!(!c.editor.documents.is_blocking());
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    assert!(Arc::ptr_eq(&before, &c.editor.document.shared_source()));
    assert_eq!(
        (
            c.editor.document.world_id(),
            c.editor.document.revision(),
            c.editor.document.state_id(),
            c.editor.document.history_len(),
            c.editor.document.is_dirty()
        ),
        state
    );
    assert_eq!(c.editor.selection.active(), Some(root));
    assert_eq!(c.editor.source, scene_path);
    assert_eq!(fs::read(&scene_path)?, scene_bytes);
    assert_eq!(fs::read(&model_meta)?, model_meta_bytes);
    let saved = c.editor.project_root().join("assets/saved.prefab");
    let content = ContentSource::from_json(&fs::read(&saved)?)?;
    assert_eq!(
        content,
        c.editor.document.capture_prefab(root, &mut c.assets)?
    );
    let id = comet_asset_pipeline::SourceMeta::load(&saved)?.id;
    let metadata = fs::read(comet_asset_pipeline::SourceMeta::path(&saved))?;

    c.context_menu("保存模型");
    c.press("保存为 Prefab...", Rect::EVERYTHING);
    set_path(&mut c, "stale.prefab");
    c.editor
        .document
        .insert_entity(EntityData::new("external edit"), &mut c.assets)?;
    let changed = c.editor.document.shared_source();
    c.frame(vec![]);
    assert!(!c.editor.documents.is_blocking());
    assert!(Arc::ptr_eq(&changed, &c.editor.document.shared_source()));
    assert!(!c.editor.project_root().join("assets/stale.prefab").exists());

    c.editor.refresh_resources()?;
    c.wait_resources()?;
    assert!(
        c.editor.resource_build.failure.is_none(),
        "{:?}",
        c.editor.resource_build.failure
    );
    assert!(
        c.editor
            .sources
            .of_kind(comet_asset_data::product::AssetKind::Prefab)
            .any(|source| source.id == id)
    );
    let snapshot = c.editor.snapshot(&mut c.assets)?;
    let draws = comet_application::extract(&snapshot, &mut c.assets, &[])?.len();
    c.press("saved.prefab", resources());
    c.press("添加到场景", resources());
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let placed = c.editor.snapshot(&mut c.assets)?;
    assert_eq!(
        comet_application::extract(&placed, &mut c.assets, &[])?.len(),
        draws * 2
    );
    assert!(
        c.editor
            .document
            .shared_source()
            .instances
            .iter()
            .any(|instance| instance.prefab == id)
    );
    assert_eq!(
        fs::read(comet_asset_pipeline::SourceMeta::path(&saved))?,
        metadata
    );
    assert_eq!(fs::read(&scene_path)?, scene_bytes);
    c.editor.start(&mut c.assets)?;
    assert!(c.editor.begin_save_prefab(root, &mut c.assets).is_err());
    Ok(())
}
