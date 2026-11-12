use super::*;
use anyhow::Result;
use comet_editor_core::Document;
use comet_scene::{ContentSource, PrefabInstance, SceneSnapshot};
use egui::{Event, PointerButton, Pos2, Rect, Vec2};
use std::fs;
use std::sync::Arc;

struct Fixture {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    selected: EntityId,
    clips: [AssetId; 2],
    ctx: egui::Context,
}
impl Fixture {
    fn new() -> Result<Self> {
        let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
        let bytes = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0\x77\x01\0\x02\0\x10\0data\x04\0\0\0\0\0\x01\0";
        for name in ["first.wav", "second.wav"] {
            fs::write(editor.project_root().join("assets").join(name), bytes)?;
        }
        comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
        assets.refresh()?;
        let clips = ["first.wav", "second.wav"].map(|name| {
            comet_asset_pipeline::SourceMeta::load(&editor.project_root().join("assets").join(name))
                .unwrap()
                .id
        });
        let entity = EntityData::new("声音实体");
        let selected = entity.id;
        editor.document = Document::open(SceneSnapshot {
            version: comet_scene::scene::SCENE_VERSION,
            entities: vec![entity],
        })?;
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.style_mut_of(theme, |style| style.animation_time = 0.);
        }
        Ok(Self {
            _temp: temp,
            editor,
            assets,
            selected,
            clips,
            ctx,
        })
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        let entity = self
            .editor
            .snapshot(&mut self.assets)
            .unwrap()
            .entities
            .into_iter()
            .find(|entity| entity.id == self.selected)
            .unwrap();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.))),
                events,
                ..Default::default()
            },
            |ui| draw(&mut self.editor, ui, &mut self.assets, &entity),
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
    fn choose(&mut self, option: &str) {
        let output = self.frame(vec![]);
        let row = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "音频片段")
            .unwrap()
            .1;
        let anchor = texts(&output)
            .into_iter()
            .find(|(_, p)| (p.y - row.y).abs() < 2. && p.x > row.x + 20.)
            .unwrap()
            .1;
        self.click(anchor);
        let output = self.frame(vec![]);
        let selected = texts(&output)
            .into_iter()
            .find(|(text, p)| text.contains(option) && p.y > anchor.y + 3.)
            .unwrap()
            .1;
        self.click(selected);
    }
    fn source(&mut self) -> Option<AudioSource> {
        self.editor
            .snapshot(&mut self.assets)
            .unwrap()
            .entities
            .into_iter()
            .find(|entity| entity.id == self.selected)
            .unwrap()
            .audio_source
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
fn escape() -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn collect(shape: &egui::epaint::Shape, result: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => result.push((
                text.galley.text().into(),
                text.pos + text.galley.rect.center().to_vec2(),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, result);
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, &mut result);
    }
    result
}
fn label(output: &egui::FullOutput, name: &str) -> Pos2 {
    texts(output)
        .into_iter()
        .find(|(text, _)| text == name)
        .unwrap_or_else(|| panic!("missing {name}: {:?}", texts(output)))
        .1
}
fn slider(output: &egui::FullOutput) -> Pos2 {
    // The slider track precedes its numeric value and label in this horizontal row.
    let row = label(output, "音量");
    Pos2::new(58., row.y)
}

fn distance_control(output: &egui::FullOutput, name: &str) -> Pos2 {
    let row = label(output, name);
    texts(output)
        .into_iter()
        .find(|(_, position)| (position.y - row.y).abs() < 2. && position.x > row.x + 20.)
        .unwrap()
        .1
}

#[test]
fn accepted_audio_metadata_stays_lazy_and_authored_controls_keep_transactions_and_savepoints()
-> Result<()> {
    let mut f = Fixture::new()?;
    for id in f
        .assets
        .index()
        .products
        .keys()
        .copied()
        .collect::<Vec<_>>()
    {
        f.assets.evict(id);
    }
    let original = f.editor.document.shared_source();
    for _ in 0..8 {
        f.frame(vec![]);
    }
    f.choose("first.wav");
    assert_eq!(
        f.assets.resident_count(),
        0,
        "idle and metadata selection must not read PCM products"
    );
    assert!(Arc::ptr_eq(&original, &f.editor.document.shared_source()));
    f.frame(vec![escape()]);
    assert!(
        texts(&f.frame(vec![]))
            .iter()
            .any(|(text, _)| text == "选择音频")
    );
    f.choose("first.wav");
    f.press("添加音频源");
    assert_eq!(f.source().unwrap().clip, f.clips[0]);

    let history = f.editor.document.history_len();
    let output = f.frame(vec![]);
    let start = slider(&output);
    let end = start + Vec2::new(-25., 0.);
    f.frame(vec![Event::PointerMoved(start), button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    assert!(f.editor.document.is_editing());
    f.frame(vec![escape()]);
    f.frame(vec![button(end, false)]);
    assert_eq!(f.source().unwrap().gain, 1.);
    assert_eq!(f.editor.document.history_len(), history);
    let output = f.frame(vec![]);
    let start = slider(&output);
    let end = start + Vec2::new(-20., 0.);
    f.frame(vec![Event::PointerMoved(start), button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    f.frame(vec![button(end, false)]);
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), history + 1);
    let gain = f.source().unwrap().gain;
    assert!(gain < 1.);
    f.press("空间音效");
    assert!(f.source().unwrap().spatial);
    let before_distance = f.editor.document.shared_source();
    let history = f.editor.document.history_len();
    let output = f.frame(vec![]);
    let start = distance_control(&output, "开始衰减距离（米）");
    let end = start + Vec2::new(12., 0.);
    f.frame(vec![Event::PointerMoved(start), button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    assert!(f.editor.document.is_editing());
    assert_ne!(f.source().unwrap().min_distance, 1.);
    f.frame(vec![escape()]);
    f.frame(vec![button(end, false)]);
    assert_eq!(f.editor.document.shared_source(), before_distance);
    assert_eq!(f.editor.document.history_len(), history);
    let output = f.frame(vec![]);
    let start = distance_control(&output, "最大距离（米）");
    let end = start + Vec2::new(15., 0.);
    f.frame(vec![Event::PointerMoved(start), button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    f.frame(vec![button(end, false)]);
    let far = f.source().unwrap().max_distance;
    assert!(far > 30.);
    assert_eq!(f.editor.document.history_len(), history + 1);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), before_distance);
    f.editor.document.redo()?;
    f.press("空间音效");
    assert!(!f.source().unwrap().spatial);
    assert_eq!(
        f.source().unwrap().max_distance,
        far,
        "switching back to 2d preserves the configured spatial profile"
    );
    f.press("循环");
    f.press("启动时播放");
    f.choose("second.wav");
    let configured = f.source().unwrap();
    assert_eq!(configured.clip, f.clips[1]);
    assert_eq!(configured.gain, gain);
    assert!(configured.looping && configured.autoplay);
    assert_eq!(configured.max_distance, far);
    f.editor.save(&mut f.assets)?;
    let saved = f.editor.document.shared_source();
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), saved);
    f.press("移除音频源");
    assert!(f.source().is_none());
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), saved);
    assert!(!f.editor.document.is_dirty());
    // Source renames are visible only after the new product index has been accepted.
    let dir = f.editor.project_root().join("assets");
    fs::rename(dir.join("second.wav"), dir.join("renamed.wav"))?;
    fs::rename(dir.join("second.wav.meta"), dir.join("renamed.wav.meta"))?;
    assert!(
        texts(&f.frame(vec![]))
            .iter()
            .any(|(text, _)| text == "second.wav")
    );
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    assert!(
        texts(&f.frame(vec![]))
            .iter()
            .any(|(text, _)| text == "renamed.wav")
    );
    Ok(())
}

#[test]
fn direct_prefab_controls_only_override_changed_fields_and_play_buttons_keep_authoring_read_only()
-> Result<()> {
    let mut f = Fixture::new()?;
    let mut entity = EntityData::new("Prefab声音");
    let source_entity = entity.id;
    entity.audio_source = Some(AudioSource::new(f.clips[0]));
    let prefab = ContentSource {
        base: SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        ..Default::default()
    };
    let path = f.editor.project_root().join("assets/sound.prefab");
    fs::write(&path, prefab.to_json()?)?;
    comet_asset_pipeline::prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let prefab_id = comet_asset_pipeline::SourceMeta::load(&path)?.id;
    let instance = comet_core::InstanceId::new();
    f.editor.document = Document::open_source(ContentSource {
        instances: vec![PrefabInstance {
            translation: [0.; 3],
            id: instance,
            prefab: prefab_id,
            parent: None,
            overrides: vec![],
        }],
        ..Default::default()
    })?;
    f.selected = f.editor.snapshot(&mut f.assets)?.entities[0].id;
    let original = f.editor.document.shared_source();
    let output = f.frame(vec![]);
    assert!(!texts(&output).iter().any(|(text, _)| text == "移除音频源"));
    f.press("循环");
    let configured = f.editor.document.shared_source();
    assert_eq!(configured.instances[0].overrides.len(), 1);
    let change = &configured.instances[0].overrides[0];
    assert_eq!(change.entity, source_entity);
    assert_eq!(change.field, schema::AUDIO_LOOPING);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    f.press("空间音效");
    let spatial = f.editor.document.shared_source();
    assert_eq!(spatial.instances[0].overrides.len(), 2);
    assert!(
        spatial.instances[0]
            .overrides
            .iter()
            .any(|change| change.entity == source_entity
                && change.field == schema::AUDIO_SPATIAL
                && change.value == FieldValue::Bool(true))
    );
    let output = f.frame(vec![]);
    let start = distance_control(&output, "最大距离（米）");
    let end = start + Vec2::new(10., 0.);
    f.frame(vec![Event::PointerMoved(start), button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    f.frame(vec![button(end, false)]);
    assert_eq!(
        f.editor.document.shared_source().instances[0]
            .overrides
            .len(),
        3
    );
    assert!(
        f.editor.document.shared_source().instances[0]
            .overrides
            .iter()
            .any(|change| change.entity == source_entity
                && change.field == schema::AUDIO_MAX_DISTANCE)
    );
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), spatial);
    f.editor.document.redo()?;
    f.editor.save(&mut f.assets)?;
    let direct = f.editor.document.shared_source();
    let direct_selected = f.selected;
    let nested_path = f.editor.project_root().join("assets/nested.prefab");
    fs::write(
        &nested_path,
        ContentSource {
            instances: vec![PrefabInstance {
                translation: [0.; 3],
                id: comet_core::InstanceId::new(),
                prefab: prefab_id,
                parent: None,
                overrides: vec![],
            }],
            ..Default::default()
        }
        .to_json()?,
    )?;
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
    f.selected = f.editor.snapshot(&mut f.assets)?.entities[0].id;
    let nested = f.editor.document.shared_source();
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "嵌套实例的音频源请在源 Prefab 中编辑。")
    );
    f.press("循环");
    f.press("空间音效");
    assert!(Arc::ptr_eq(&nested, &f.editor.document.shared_source()));
    assert!(
        f.editor
            .edit_audio_source(
                f.selected,
                Some(AudioSource::new(f.clips[1])),
                &mut f.assets
            )
            .is_err()
    );
    f.editor.document = Document::open_source((*direct).clone())?;
    f.selected = direct_selected;
    f.editor.start(&mut f.assets)?;
    f.editor.play.as_mut().unwrap().set_paused(true);
    let before = f.editor.document.shared_source();
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "恢复运行后才会出声。")
    );
    f.press("循环");
    f.press("空间音效");
    assert!(Arc::ptr_eq(&before, &f.editor.document.shared_source()));
    f.press("播放声音");
    let frame = f.editor.play.as_mut().unwrap().audio_frame()?.unwrap();
    assert!(frame.paused);
    assert_eq!(frame.voices[0].generation, 1);
    assert!(frame.voices[0].playing);
    f.press("停止声音");
    assert!(
        !f.editor
            .play
            .as_mut()
            .unwrap()
            .audio_frame()?
            .unwrap()
            .voices[0]
            .playing
    );
    f.editor.stop();
    assert!(Arc::ptr_eq(&before, &f.editor.document.shared_source()));
    f.editor
        .documents
        .open_browser(crate::documents::BrowserMode::Open);
    f.press("循环");
    assert!(Arc::ptr_eq(&before, &f.editor.document.shared_source()));
    f.editor.documents = Default::default();
    f.editor.refresh_resources()?;
    f.press("循环");
    assert!(Arc::ptr_eq(&before, &f.editor.document.shared_source()));
    Ok(())
}
