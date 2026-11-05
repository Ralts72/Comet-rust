use super::*;
use comet_scene::{
    ContentSource, SceneSnapshot,
    schema::{self, FieldValue},
};
use std::{path::PathBuf, sync::Arc};

struct Case {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    number: u64,
    prefab: PathBuf,
    root: comet_core::EntityId,
    meta: Vec<u8>,
}

impl Case {
    fn new() -> Result<Self> {
        let (temp, mut editor, mut assets) = fixture()?;
        let root = EntityData::new("Prefab 作者根");
        let id = root.id;
        let prefab = editor.project_root().join("assets/props.prefab");
        fs::write(
            &prefab,
            ContentSource {
                base: SceneSnapshot {
                    entities: vec![root],
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_json()?,
        )?;
        // A real imported GLB belongs in the resource list, but cannot be opened as an author document.
        let directory = editor.project_root().join("assets");
        let mut gltf: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("showcase.gltf"))?)?;
        gltf["buffers"][0].as_object_mut().unwrap().remove("uri");
        let mut json = serde_json::to_vec(&gltf)?;
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = fs::read(directory.join("showcase.bin"))?;
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend((json.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(json);
        glb.extend((bin.len() as u32).to_le_bytes());
        glb.extend(b"BIN\0");
        glb.extend(bin);
        fs::write(directory.join("copy.glb"), glb)?;
        comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
        assets.refresh()?;
        editor.sources = comet_asset_pipeline::SourceCatalog::read(
            &comet_core::ProjectPaths::open(editor.project_root())?,
            assets.index(),
        )?;
        let meta = fs::read(comet_asset_pipeline::SourceMeta::path(&prefab))?;
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
            prefab,
            root: id,
            meta,
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

    fn click(&mut self, position: Pos2) -> egui::FullOutput {
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(position), button(true)]);
        self.frame(vec![button(false)])
    }

    fn open_document(&mut self, path: &str) -> egui::FullOutput {
        self.frame(vec![key(egui::Key::O, egui::Modifiers::COMMAND)]);
        let output = self.frame(vec![]);
        self.click(label(&output, path, dialog()));
        let output = self.frame(vec![]);
        self.click(label(&output, "打开", dialog()))
    }

    fn resource(&mut self, text: &str) -> egui::FullOutput {
        let output = self.frame(vec![]);
        self.click(label(&output, text, resources()))
    }

    fn new_prefab_dialog(&mut self) -> egui::FullOutput {
        let output = self.frame(vec![]);
        self.click(label(&output, "新建Prefab", resources()));
        self.frame(vec![])
    }

    fn choose_model_asset(&mut self, field: &str, option: &str) {
        let output = self.frame(vec![]);
        let anchor = label(&output, &format!("选择{field}"), Rect::EVERYTHING);
        self.click(anchor);
        let output = self.frame(vec![]);
        let position = texts(&output, Rect::EVERYTHING)
            .into_iter()
            .find(|(text, point)| text.contains(option) && point.y > anchor.y + 3.)
            .map(|(_, point)| point)
            .unwrap_or_else(|| panic!("missing {option}; {:?}", texts(&output, Rect::EVERYTHING)));
        self.click(position);
    }
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn dialog() -> Rect {
    Rect::from_min_max(Pos2::new(250., 60.), Pos2::new(1150., 650.))
}
fn resources() -> Rect {
    Rect::from_min_max(Pos2::new(175., 660.), Pos2::new(1000., 775.))
}

fn texts(output: &egui::FullOutput, region: Rect) -> Vec<(String, Pos2)> {
    fn find(shape: &egui::epaint::Shape, region: Rect, result: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => {
                let point = text.pos + text.galley.rect.center().to_vec2();
                if region.contains(point) {
                    result.push((text.galley.job.text.clone(), point));
                }
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    find(shape, region, result);
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        find(&shape.shape, region, &mut result);
    }
    result
}

fn label(output: &egui::FullOutput, value: &str, region: Rect) -> Pos2 {
    texts(output, region)
        .into_iter()
        .find(|(text, _)| {
            text == value
                || (value.ends_with(".scene")
                    || value.ends_with(".prefab")
                    || value.ends_with(".gltf")
                    || value.ends_with(".glb"))
                    && text.ends_with(value)
        })
        .map(|(_, point)| point)
        .unwrap_or_else(|| panic!("missing {value}; {:?}", texts(output, region)))
}

#[test]
fn open_document_lists_native_sources_and_prefab_preview_keeps_authoring_and_scene_export_separate()
-> Result<()> {
    let mut c = Case::new()?;
    c.frame(vec![]);
    c.frame(vec![key(egui::Key::O, egui::Modifiers::COMMAND)]);
    let output = c.frame(vec![]);
    let names = texts(&output, dialog());
    assert!(names.iter().any(|(text, _)| text == "场景  main.scene"));
    assert!(names.iter().any(|(text, _)| text == "Prefab  props.prefab"));
    assert!(
        !names
            .iter()
            .any(|(text, _)| text.contains("showcase.gltf") || text.contains("copy.glb"))
    );
    c.click(label(&output, "props.prefab", dialog()));
    let output = c.frame(vec![]);
    c.click(label(&output, "打开", dialog()));
    assert_eq!(c.editor.source, c.prefab);
    assert_eq!(c.editor.document_kind_label(), "Prefab");
    let output = c.frame(vec![]);
    assert!(
        texts(&output, Rect::EVERYTHING)
            .iter()
            .any(|(text, _)| text == "编辑Prefab")
    );
    assert!(
        texts(&output, Rect::EVERYTHING)
            .iter()
            .any(|(text, _)| text == "Prefab根节点")
    );
    let original = c.editor.document.shared_source();
    let bytes = fs::read(&c.prefab)?;
    c.click(label(&output, "运行", Rect::EVERYTHING));
    let world = c.editor.world();
    assert_ne!(world, c.editor.document.world_id());
    c.editor.edit(
        c.root,
        schema::TRANSLATION,
        FieldValue::Vec3([9., 8., 7.]),
        &mut c.assets,
    )?;
    assert!(Arc::ptr_eq(&original, &c.editor.document.shared_source()));
    let output = c.frame(vec![]);
    assert!(
        texts(&output, Rect::EVERYTHING)
            .iter()
            .any(|(text, _)| text == "运行预览 · 作者Prefab")
    );
    c.click(label(&output, "停止", Rect::EVERYTHING));
    assert!(c.editor.play.is_none());
    assert_eq!(
        c.editor.snapshot(&mut c.assets)?.entities[0]
            .transform
            .translation,
        [0.; 3]
    );
    assert_eq!(fs::read(&c.prefab)?, bytes);
    assert_eq!(
        fs::read(comet_asset_pipeline::SourceMeta::path(&c.prefab))?,
        c.meta
    );
    let output = c.frame(vec![]);
    c.click(label(&output, "导出游戏", Rect::EVERYTHING));
    c.frame(vec![]);
    assert!(
        c.editor.export.open,
        "Prefab authors can still export a selected Scene"
    );
    let prefab_asset = comet_asset_pipeline::SourceMeta::load(&c.prefab)?.id;
    assert_ne!(c.editor.export.entry_scene(), Some(prefab_asset));
    let scene = c
        .editor
        .sources
        .of_kind(comet_asset_data::product::AssetKind::Scene)
        .next()
        .unwrap()
        .id;
    assert_eq!(c.editor.export.entry_scene(), Some(scene));
    Ok(())
}

#[test]
fn edit_source_uses_confirmation_then_saves_only_the_open_native_prefab() -> Result<()> {
    let mut c = Case::new()?;
    let scene = c.editor.source.clone();
    let scene_bytes = fs::read(&scene)?;
    let id = c
        .editor
        .snapshot(&mut c.assets)?
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    c.editor.edit(
        id,
        schema::TRANSLATION,
        FieldValue::Vec3([2., 3., 4.]),
        &mut c.assets,
    )?;
    let preview = c.editor.document.shared_source();
    c.resource("props.prefab");
    let output = c.frame(vec![]);
    c.click(label(&output, "编辑源", resources()));
    assert!(c.editor.documents.pending && c.editor.document.is_editing());
    assert!(Arc::ptr_eq(&preview, &c.editor.document.shared_source()));
    let output = c.frame(vec![]);
    assert!(
        texts(&output, dialog())
            .iter()
            .any(|(text, _)| text.contains("当前场景「main.scene」"))
    );
    c.click(label(&output, "取消", dialog()));
    assert!(!c.editor.documents.is_blocking());
    assert_eq!(c.editor.source, scene);
    assert!(Arc::ptr_eq(&preview, &c.editor.document.shared_source()));
    assert_eq!(fs::read(&scene)?, scene_bytes);
    let output = c.frame(vec![]);
    c.click(label(&output, "编辑源", resources()));
    let output = c.frame(vec![]);
    c.click(label(&output, "保存并继续", dialog()));
    assert_eq!(c.editor.source, c.prefab);
    assert_eq!(
        comet_editor_core::Document::load(&scene)?.shared_source(),
        preview
    );
    let saved_scene = fs::read(&scene)?;
    let output = c.frame(vec![]);
    c.click(label(
        &output,
        "Prefab 作者根",
        Rect::from_min_max(Pos2::ZERO, Pos2::new(220., 660.)),
    ));
    let output = c.frame(vec![]);
    let start = texts(
        &output,
        Rect::from_min_max(Pos2::new(1000., 40.), Pos2::new(1280., 660.)),
    )
    .into_iter()
    .find(|(text, _)| text.starts_with("X "))
    .unwrap()
    .1;
    let end = start + Vec2::new(40., 0.);
    c.frame(vec![
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    c.frame(vec![egui::Event::PointerMoved(end)]);
    c.frame(vec![egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    }]);
    assert!(c.editor.document.is_dirty());
    let authored = c.editor.document.shared_source();
    let output = c.frame(vec![]);
    c.click(label(&output, "撤销", Rect::EVERYTHING));
    assert!(!c.editor.document.is_dirty());
    assert_eq!(
        c.editor.document.shared_source().base.entities[0]
            .transform
            .translation,
        [0.; 3]
    );
    let output = c.frame(vec![]);
    c.click(label(&output, "重做", Rect::EVERYTHING));
    assert_eq!(c.editor.document.shared_source(), authored);
    c.frame(vec![key(egui::Key::S, egui::Modifiers::COMMAND)]);
    assert!(!c.editor.document.is_dirty());
    assert_eq!(
        comet_editor_core::Document::load(&c.prefab)?.shared_source(),
        authored
    );
    assert_eq!(authored.base.entities[0].id, c.root);
    assert!(authored.base.entities[0].transform.translation[0] > 0.);
    assert_eq!(fs::read(&scene)?, saved_scene);
    assert_eq!(
        fs::read(comet_asset_pipeline::SourceMeta::path(&c.prefab))?,
        c.meta
    );
    Ok(())
}

#[test]
fn imported_models_keep_placement_without_an_edit_source_action() -> Result<()> {
    let mut c = Case::new()?;
    for name in ["showcase.gltf", "copy.glb"] {
        c.resource(name);
        let original = c.editor.document.shared_source();
        let path = c.editor.project_root().join("assets").join(name);
        let bytes = fs::read(&path)?;
        let output = c.frame(vec![]);
        c.click(label(&output, "编辑源", resources()));
        assert!(!c.editor.documents.pending);
        assert!(Arc::ptr_eq(&original, &c.editor.document.shared_source()));
        let output = c.frame(vec![]);
        c.click(label(&output, "添加到场景", resources()));
        assert!(c.editor.document.shared_source().instances.len() > original.instances.len());
        assert_eq!(fs::read(path)?, bytes);
    }
    Ok(())
}

#[test]
fn failed_prefab_save_keeps_its_preview_and_play_until_switch_retry_succeeds() -> Result<()> {
    let mut c = Case::new()?;
    c.open_document("props.prefab");
    assert_eq!(c.editor.source, c.prefab);
    c.editor.edit(
        c.root,
        schema::TRANSLATION,
        FieldValue::Vec3([3., 4., 5.]),
        &mut c.assets,
    )?;
    let output = c.frame(vec![]);
    c.click(label(&output, "运行", Rect::EVERYTHING));
    let preview = c.editor.document.shared_source();
    let world = c.editor.world();
    c.open_document("main.scene");
    assert!(c.editor.documents.pending);
    let output = c.frame(vec![]);
    assert!(
        texts(&output, dialog())
            .iter()
            .any(|(text, _)| text.contains("当前Prefab「props.prefab」"))
    );
    let backup = c.prefab.with_extension("backup");
    fs::rename(&c.prefab, &backup)?;
    fs::create_dir(&c.prefab)?;
    c.click(label(&output, "保存并继续", dialog()));
    assert_eq!(c.editor.source, c.prefab);
    assert_eq!(c.editor.world(), world);
    assert!(c.editor.documents.pending);
    assert!(Arc::ptr_eq(&preview, &c.editor.document.shared_source()));
    let output = c.frame(vec![]);
    assert!(
        texts(&output, dialog())
            .iter()
            .any(|(text, _)| text.contains("当前Prefab仍保留"))
    );
    fs::remove_dir(&c.prefab)?;
    fs::rename(backup, &c.prefab)?;
    c.click(label(&output, "保存并继续", dialog()));
    assert!(c.editor.source.ends_with("main.scene"));
    assert!(c.editor.play.is_none());
    assert_eq!(
        comet_editor_core::Document::load(&c.prefab)?.shared_source(),
        preview
    );
    assert_eq!(
        fs::read(comet_asset_pipeline::SourceMeta::path(&c.prefab))?,
        c.meta
    );
    Ok(())
}

#[test]
fn creating_a_prefab_through_resources_can_cancel_then_author_save_and_undo_a_model() -> Result<()>
{
    let mut c = Case::new()?;
    let scene = c.editor.source.clone();
    let scene_bytes = fs::read(&scene)?;
    let output = c.new_prefab_dialog();
    assert!(
        texts(&output, dialog())
            .iter()
            .any(|(text, _)| text == "new.prefab")
    );
    c.click(label(&output, "取消", dialog()));
    assert_eq!(c.editor.source, scene);
    assert!(!scene.parent().unwrap().join("new.prefab").exists());

    let path = scene.parent().unwrap().join("authored.prefab");
    let reserved = comet_asset_pipeline::SourceMeta {
        version: 1,
        id: comet_core::AssetId::new(),
    };
    let metadata = serde_json::to_vec_pretty(&reserved)?;
    fs::write(comet_asset_pipeline::SourceMeta::path(&path), &metadata)?;
    let output = c.new_prefab_dialog();
    c.click(label(&output, "new.prefab", dialog()));
    c.frame(vec![
        key(egui::Key::A, egui::Modifiers::COMMAND),
        egui::Event::Text("authored.prefab".into()),
    ]);
    let output = c.frame(vec![]);
    c.click(label(&output, "创建", dialog()));
    assert_eq!(c.editor.source, path);
    assert_eq!(c.editor.document.prefab_asset(), Some(reserved.id));
    assert!(c.editor.document.shared_source().base.entities.is_empty());
    assert!(!c.editor.document.is_dirty());
    assert_eq!(c.editor.document.history_len(), 0);

    let output = c.frame(vec![]);
    c.click(label(&output, "新建对象", Rect::EVERYTHING));
    let empty_object = c.editor.document.shared_source();
    let entity = empty_object.base.entities[0].id;
    assert!(empty_object.base.entities[0].model.is_none());
    c.choose_model_asset("网格", "AmberTwoMaterials");
    c.choose_model_asset("材质", "Porcelain");
    let output = c.frame(vec![]);
    c.click(label(&output, "添加模型", Rect::EVERYTHING));
    assert!(c.editor.error.is_none(), "{:?}", c.editor.error);
    let authored = c.editor.document.shared_source();
    assert_eq!(authored.base.entities[0].id, entity);
    assert!(authored.base.entities[0].model.is_some());
    assert_eq!(c.editor.document.history_len(), 2);
    assert_eq!(
        comet_application::extract(&c.editor.snapshot(&mut c.assets)?, &mut c.assets, &[])?.len(),
        1
    );
    c.frame(vec![key(egui::Key::S, egui::Modifiers::COMMAND)]);
    assert!(!c.editor.document.is_dirty());
    let saved = fs::read(&path)?;
    assert_eq!(
        comet_editor_core::Document::load(&path)?.shared_source(),
        authored
    );
    assert_eq!(
        fs::read(comet_asset_pipeline::SourceMeta::path(&path))?,
        metadata
    );

    let output = c.frame(vec![]);
    c.click(label(&output, "撤销", Rect::EVERYTHING));
    assert_eq!(c.editor.document.shared_source(), empty_object);
    let output = c.frame(vec![]);
    c.click(label(&output, "撤销", Rect::EVERYTHING));
    assert!(c.editor.document.shared_source().base.entities.is_empty());
    assert_eq!(
        fs::read(&path)?,
        saved,
        "undo does not write the author file"
    );
    for _ in 0..2 {
        let output = c.frame(vec![]);
        c.click(label(&output, "重做", Rect::EVERYTHING));
    }
    assert_eq!(c.editor.document.shared_source(), authored);
    assert!(!c.editor.document.is_dirty());

    c.resource("authored.prefab");
    let output = c.frame(vec![]);
    c.click(label(&output, "添加到Prefab", resources()));
    let output = c.frame(vec![]);
    let position = label(&output, "authored.prefab", resources());
    c.click(position);
    c.click(position);
    assert!(
        c.editor.error.is_none(),
        "self placement is disabled before issuing a command"
    );
    assert!(Arc::ptr_eq(&authored, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), 2);
    assert_eq!(fs::read(&scene)?, scene_bytes);
    assert_eq!(fs::read(&path)?, saved);
    Ok(())
}
