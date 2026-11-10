use super::*;
use comet_core::{EntityId, InstanceId};
use comet_editor_core::Document;
use comet_scene::{BodyMotion, ContentSource, PhysicsBody, PrefabInstance, SceneSnapshot};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

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

impl Case {
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
        .1
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
    fn find(shape: &egui::epaint::Shape, label: &str, region: Rect) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text)
                if text.galley.job.text == label
                    || text.galley.job.text.ends_with(label)
                    || text.galley.job.text.starts_with(&format!("{label} ")) =>
            {
                let point = text.pos + text.galley.rect.center().to_vec2();
                region.contains(point).then_some(point)
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| find(shape, label, region))
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label, region))
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
        .place_resource(choice.id, "模型容器", &mut c.assets)?;
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
