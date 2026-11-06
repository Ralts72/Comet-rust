use crate::{session::Editor, test_support::fixture};
use anyhow::{Context, Result};
use comet_application::FrameContext;
use comet_asset::AssetStore;
use comet_core::{
    EntityId,
    math::{EulerRot, Quat},
};
use comet_editor_core::Document;
use comet_scene::{EntityData, SceneSnapshot, Transform};
use egui::{Event, PointerButton, Pos2, Vec2};
mod configurations;
mod model_assets;
mod scalars;

struct Group {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    ids: [EntityId; 2],
    number: u64,
}
impl Group {
    fn new() -> Result<Self> {
        let (temp, mut editor, mut assets) = fixture()?;
        let model = editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .find_map(|entity| entity.model.clone())
            .context("renderable fixture model")?;
        let mut parent = EntityData::new("主层级");
        parent.transform = Transform {
            translation: [-1., 1., 0.],
            rotation: Quat::from_euler(EulerRot::XYZ, 0.1, 0.2, 0.3).to_array(),
            scale: [1., 1., 1.],
        };
        parent.model = Some(model.clone());
        parent.model.as_mut().unwrap().surface.roughness = 0.2;
        let mut child = EntityData::new("主实体");
        child.parent = Some(parent.id);
        child.transform = Transform {
            translation: [1., 2., 0.5],
            rotation: Quat::from_euler(EulerRot::XYZ, 0.4, 0.5, 0.6).to_array(),
            scale: [1.5, 1.5, 1.5],
        };
        child.model = Some(model);
        child.model.as_mut().unwrap().surface.roughness = 0.8;
        let ids = [parent.id, child.id];
        editor.document = Document::open(SceneSnapshot {
            entities: vec![parent, child],
            ..Default::default()
        })?;
        editor.selection.replace(ids);
        let ctx = egui::Context::default();
        comet_application::configure_ui(&ctx);
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        Ok(Self {
            _temp: temp,
            editor,
            assets,
            ctx,
            ids,
            number: 0,
        })
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        let context = FrameContext {
            number: self.number,
            dt: 1. / 60.,
            active: true,
            activity_changed: false,
            target_generation: 1,
            size: [1280, 800],
            smoke: false,
        };
        self.number += 1;
        crate::ui::tests::frame_with_output(
            &self.ctx,
            &mut self.editor,
            &mut self.assets,
            context,
            events,
        )
        .1
    }
    fn transforms(&self) -> [Transform; 2] {
        let source = self.editor.document.shared_source();
        self.ids.map(|id| {
            source
                .base
                .entities
                .iter()
                .find(|entity| entity.id == id)
                .unwrap()
                .transform
                .clone()
        })
    }
    fn drag(&mut self, field: &str, axis: &str, distance: f32) -> Result<(Pos2, Pos2)> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let start = control(&output, field, axis)?;
        let end = start + Vec2::new(distance, 0.);
        self.frame(vec![Event::PointerMoved(start), button(start, true)]);
        self.frame(vec![Event::PointerMoved(end)]);
        Ok((start, end))
    }
    fn type_value(&mut self, field: &str, axis: &str, value: &str) -> Result<()> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let pos = control(&output, field, axis)?;
        self.type_at(pos, value);
        Ok(())
    }
    fn type_at(&mut self, pos: Pos2, value: &str) {
        self.frame(vec![Event::PointerMoved(pos), button(pos, true)]);
        self.frame(vec![button(pos, false)]);
        self.frame(vec![
            key(egui::Key::A, true, egui::Modifiers::COMMAND),
            key(egui::Key::A, false, egui::Modifiers::COMMAND),
            Event::Text(value.into()),
        ]);
    }
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn collect(shape: &egui::epaint::Shape, result: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::epaint::Shape::Text(text) => result.push((
                text.galley.job.text.clone(),
                text.pos + text.galley.rect.center().to_vec2(),
            )),
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().for_each(|shape| collect(shape, result))
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    output
        .shapes
        .iter()
        .for_each(|shape| collect(&shape.shape, &mut result));
    result
}
fn control(output: &egui::FullOutput, field: &str, axis: &str) -> Result<Pos2> {
    let values = texts(output);
    let heading = values
        .iter()
        .find(|(text, pos)| text.starts_with(field) && pos.x > 970.)
        .context("common property heading")?
        .1;
    values
        .iter()
        .filter(|(text, pos)| {
            text.starts_with(&format!("{axis} ")) && pos.x > 970. && pos.y > heading.y
        })
        .min_by(|(_, a), (_, b)| a.y.total_cmp(&b.y))
        .map(|(_, pos)| *pos)
        .context("common property control")
}
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn typed_common_values_commit_once_and_escape_restores_the_whole_group() -> Result<()> {
    let mut f = Group::new()?;
    f.type_value("位置", "X", "7.25")?;
    f.frame(vec![key(egui::Key::Enter, true, Default::default())]);
    f.frame(vec![key(egui::Key::Enter, false, Default::default())]);
    assert!(
        f.transforms()
            .iter()
            .all(|transform| transform.translation[0] == 7.25)
    );
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 1);
    let accepted = f.editor.document.shared_source();
    f.type_value("位置", "Y", "9.5")?;
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.shared_source(), accepted);
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 1);
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_common_properties_draws_mixed_transforms_without_authoring_changes() -> Result<()> {
    let mut f = Group::new()?;
    f.mixed_models()?;
    f.audio()?;
    let output = f.frame(vec![]);
    let mixed: Vec<_> = texts(&output)
        .into_iter()
        .filter_map(|(text, _)| {
            ["位置（多值）", "旋转（多值）", "缩放（多值）"]
                .contains(&text.as_str())
                .then_some(text)
        })
        .collect();
    assert_eq!(mixed.len(), 3);
    let scalars = ["粗糙度（多值）", "音量（多值）"];
    for label in scalars {
        assert!(
            texts(&output)
                .iter()
                .any(|(text, _)| text.starts_with(label))
        );
    }
    let flags: Vec<_> = texts(&output)
        .into_iter()
        .filter_map(|(text, _)| {
            ["循环（多值）", "启动时播放（多值）", "空间音效（多值）"]
                .contains(&text.as_str())
                .then_some(text)
        })
        .collect();
    assert_eq!(flags.len(), 3);
    let asset_fields = ["网格（多值）", "材质（多值）"];
    for label in asset_fields {
        assert!(texts(&output).iter().any(|(text, _)| text == label));
    }
    let author_file = f.editor.source.clone();
    let author_bytes = std::fs::read(&author_file)?;
    let world = f.editor.document.world_id();
    let mut selected = f.ids.to_vec();
    selected.sort();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    std::fs::create_dir_all(&directory)?;
    let report_path = directory.join("editor-common-properties.json");
    let capture = directory.join("editor-common-properties.png");
    let mut options = comet_application::HostOptions::new("common property test");
    options.headless = true;
    options.smoke = true;
    options.report = Some(report_path.clone());
    options.capture = Some(capture.clone());
    comet_application::run(options, f.assets, f.editor)?;
    let mut report: serde_json::Value = serde_json::from_slice(&std::fs::read(&report_path)?)?;
    assert_eq!(report["submitted_frames"], 8);
    assert_eq!(report["application"]["play_active"], false);
    assert_eq!(report["application"]["dirty"], false);
    assert_eq!(report["application"]["history"], 0);
    assert!(report["application"]["error"].is_null());
    assert_eq!(
        report["application"]["selected"],
        serde_json::json!(selected)
    );
    assert!(
        report["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| frame["token"]["world"] == world.to_string() && frame["draw_count"] == 2)
    );
    assert!(std::fs::metadata(&capture)?.len() > 8);
    assert_eq!(std::fs::read(author_file)?, author_bytes);
    report["common_properties"] = serde_json::json!({"mixed_fields": mixed, "scalar_fields": scalars, "mixed_flags": flags, "asset_fields": asset_fields, "source_unchanged": true});
    std::fs::write(report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

#[test]
fn grouped_inspector_changes_one_local_axis_for_parent_and_child_and_commits_one_history_entry()
-> Result<()> {
    let mut f = Group::new()?;
    let original = f.editor.document.shared_source();
    let before = f.transforms();
    let saved = std::fs::read(&f.editor.source)?;
    let (_, end) = f.drag("位置", "X", 40.)?;
    let after = f.transforms();
    assert!(f.editor.document.is_editing());
    assert_eq!(after[0].translation[0], after[1].translation[0]);
    assert!(after[1].translation[0] > before[1].translation[0]);
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(old.translation[1..], new.translation[1..]);
        assert_eq!(old.rotation, new.rotation);
        assert_eq!(old.scale, new.scale);
    }
    f.frame(vec![button(end, false)]);
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 1);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    assert_eq!(f.transforms(), after);
    assert_eq!(std::fs::read(&f.editor.source)?, saved);
    Ok(())
}

#[test]
fn grouped_rotation_and_scale_keep_other_local_axes_and_escape_retires_the_drag() -> Result<()> {
    let mut f = Group::new()?;
    let before = f.transforms();
    let (_, end) = f.drag("旋转", "Y", 20.)?;
    f.frame(vec![button(end, false)]);
    let after = f.transforms();
    let expected_y = Quat::from_array(after[1].rotation)
        .to_euler(EulerRot::XYZ)
        .1;
    for (old, new) in before.iter().zip(&after) {
        let (x, _, z) = Quat::from_array(old.rotation).to_euler(EulerRot::XYZ);
        let expected = Quat::from_euler(EulerRot::XYZ, x, expected_y, z);
        assert!((Quat::from_array(new.rotation).dot(expected).abs() - 1.).abs() < 1e-5);
        assert_eq!(old.translation, new.translation);
        assert_eq!(old.scale, new.scale);
    }
    assert!((expected_y - 0.5).abs() > 0.01);
    let accepted = f.editor.document.shared_source();
    let history = f.editor.document.history_len();
    let (_, end) = f.drag("缩放", "Z", 25.)?;
    let scaled = f.transforms();
    assert_eq!(scaled[0].scale[2], scaled[1].scale[2]);
    for (old, new) in after.iter().zip(&scaled) {
        assert_eq!(old.scale[..2], new.scale[..2]);
    }
    f.frame(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    assert_eq!(f.editor.document.shared_source(), accepted);
    assert!(!f.editor.document.is_editing());
    f.frame(vec![Event::PointerMoved(end + Vec2::new(30., 0.))]);
    f.frame(vec![button(end, false)]);
    assert_eq!(f.editor.document.shared_source(), accepted);
    assert_eq!(f.editor.document.history_len(), history);
    Ok(())
}

#[test]
fn play_common_properties_remain_read_only_and_keep_the_author_document() -> Result<()> {
    let mut f = Group::new()?;
    let before = f.editor.document.shared_source();
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "共同属性 · 2 个实体")
    );
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "运行期间暂不支持成组修改。")
    );
    let (_, end) = f.drag("位置", "X", 40.)?;
    f.frame(vec![button(end, false)]);
    assert_eq!(f.editor.document.shared_source(), before);
    let play = f.editor.snapshot(&mut f.assets)?;
    for entity in &play.entities {
        assert_eq!(
            entity.transform,
            before
                .base
                .entities
                .iter()
                .find(|old| old.id == entity.id)
                .unwrap()
                .transform
        );
    }
    f.editor.stop();
    assert_eq!(f.editor.document.shared_source(), before);
    Ok(())
}

#[test]
fn an_invalid_character_scale_rejects_the_entire_common_property_gesture() -> Result<()> {
    use comet_scene::{BodyMotion, Character, ColliderShape, PhysicsBody};
    let mut f = Group::new()?;
    let mut source = f.editor.document.source();
    let actor = source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.ids[0])
        .unwrap();
    actor.transform.rotation = Quat::IDENTITY.to_array();
    actor.physics = Some(PhysicsBody {
        motion: BodyMotion::Kinematic,
        shape: ColliderShape::Capsule {
            half_height: 0.5,
            radius: 0.3,
        },
        ..Default::default()
    });
    actor.character = Some(Character::default());
    f.editor.document = Document::open_source(source)?;
    let before = f.editor.document.shared_source();
    let (_, end) = f.drag("缩放", "X", 20.)?;
    f.frame(vec![button(end, false)]);
    assert!(f.editor.error.is_some());
    assert_eq!(f.editor.document.shared_source(), before);
    assert!(!f.editor.document.is_editing());
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}
