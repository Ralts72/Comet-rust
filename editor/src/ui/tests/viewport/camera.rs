use super::*;
use comet_application::Projection;

struct Case {
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    number: u64,
    active: bool,
}
impl Case {
    fn frame(&mut self, events: Vec<egui::Event>) -> (ViewRequest, egui::FullOutput) {
        self.number += 1;
        let mut input = context(self.number);
        input.active = self.active;
        frame_with_output(&self.ctx, &mut self.editor, &mut self.assets, input, events)
    }
    fn press(&mut self, label: &str) {
        let output = self.frame(vec![]).1;
        let pos = text_center(&output, label);
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                pointer(pos, pressed, Default::default()),
            ]);
        }
    }
    fn focus(&mut self, modifiers: egui::Modifiers) {
        let view = self.frame(vec![]).0;
        self.frame(vec![
            egui::Event::PointerMoved(view.viewport.center()),
            egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
        ]);
    }
}
fn setup() -> Result<(tempfile::TempDir, Case, comet_core::EntityId)> {
    let (temp, mut editor, assets) = fixture()?;
    let mut content = editor.document.source();
    let mut parent = EntityData::new("聚焦容器");
    parent.transform.translation = [4., 0., 0.];
    let id = parent.id;
    content.base.entities.push(parent);
    content.instances[0].parent = Some(id);
    editor.document = comet_editor_core::Document::open_source(content)?;
    editor.selection.replace([id]);
    let ctx = egui::Context::default();
    comet_application::configure_ui(&ctx);
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let mut case = Case {
        editor,
        assets,
        ctx,
        number: 0,
        active: true,
    };
    case.frame(vec![]);
    case.frame(vec![]);
    Ok((temp, case, id))
}

#[test]
fn projection_and_focus_controls_preserve_authoring_and_obey_text_play_preview_and_host_guards()
-> Result<()> {
    let (_temp, mut case, id) = setup()?;
    let original = case.editor.document.shared_source();
    let bytes = fs::read(&case.editor.source)?;
    let direction = (case.editor.camera.eye - case.editor.camera.target).normalize();
    case.focus(egui::Modifiers::NONE);
    assert!(case.editor.camera.target.x > 3.);
    let fitted = case.editor.camera;
    case.press("投影：3D");
    case.press("2D 正交");
    assert!(matches!(
        case.editor.camera.projection,
        Projection::Orthographic { .. }
    ));
    assert!(
        (case.editor.camera.eye - case.editor.camera.target)
            .normalize()
            .distance(Vec3::Z)
            < 1e-5
    );
    case.focus(egui::Modifiers::NONE);
    assert!(case.editor.error.is_none(), "{:?}", case.editor.error);
    case.press("投影：2D");
    case.press("3D 透视");
    assert_eq!(case.editor.camera.projection, Projection::Perspective);
    assert!(
        (case.editor.camera.eye - case.editor.camera.target)
            .normalize()
            .distance(direction)
            < 1e-5
    );
    assert_eq!(case.editor.selection.ids(), [id]);
    assert_eq!(case.editor.camera.target, fitted.target);

    case.editor.camera.eye += Vec3::X * 10.;
    case.editor.camera.target += Vec3::X * 10.;
    let before = case.editor.camera;
    case.focus(egui::Modifiers::COMMAND);
    assert_eq!(case.editor.camera, before);
    case.press("搜索实体...");
    assert!(case.ctx.text_edit_focused());
    case.focus(egui::Modifiers::NONE);
    assert_eq!(case.editor.camera, before, "text input owns F");
    if let Some(id) = case.ctx.memory(|memory| memory.focused()) {
        case.ctx.memory_mut(|memory| memory.surrender_focus(id));
    }
    case.active = false;
    case.focus(egui::Modifiers::NONE);
    assert_eq!(case.editor.camera, before);
    case.active = true;
    case.editor.document.begin()?;
    case.focus(egui::Modifiers::NONE);
    assert_eq!(case.editor.camera, before);
    assert!(case.editor.document.is_editing());
    case.editor.document.cancel()?;
    case.editor.start(&mut case.assets)?;
    case.focus(egui::Modifiers::NONE);
    assert_eq!(case.editor.camera, before, "Play keeps game input");
    case.editor.stop();
    assert!(Arc::ptr_eq(
        &original,
        &case.editor.document.shared_source()
    ));
    assert_eq!(case.editor.document.history_len(), 0);
    assert!(!case.editor.document.is_dirty());
    assert_eq!(fs::read(&case.editor.source)?, bytes);
    Ok(())
}

struct Capture {
    editor: Editor,
}
impl Application for Capture {
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        input: &FrameContext,
    ) -> Result<ViewRequest> {
        let view = self.editor.frame(ui, assets, input)?;
        assert!(matches!(
            view.camera.projection,
            Projection::Orthographic { .. }
        ));
        Ok(view)
    }
    fn feedback(&mut self, feedback: &FrameFeedback) {
        self.editor.feedback(feedback);
    }
    fn report(&mut self) -> serde_json::Value {
        let mut report = self.editor.report();
        report["camera_projection"] = "orthographic".into();
        report["camera_target"] = serde_json::json!(self.editor.camera.target.to_array());
        report
    }
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_orthographic_camera_frames_selected_prefab_descendants_without_editing() -> Result<()> {
    let (_temp, mut case, _) = setup()?;
    let id = case
        .editor
        .snapshot(&mut case.assets)?
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    case.editor.selection.replace([id]);
    case.press("投影：3D");
    case.press("2D 正交");
    case.focus(egui::Modifiers::NONE);
    assert!(case.editor.error.is_none(), "{:?}", case.editor.error);
    let source = case.editor.source.clone();
    let bytes = fs::read(&source)?;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    let mut options = comet_application::HostOptions::new("Comet · 正交相机聚焦");
    options.headless = true;
    options.smoke = true;
    options.capture = Some(out.join("editor-camera.png"));
    options.report = Some(out.join("editor-camera.json"));
    comet_application::run(
        options,
        case.assets,
        Capture {
            editor: case.editor,
        },
    )?;
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("editor-camera.json"))?)?;
    assert_eq!(report["submitted_frames"], 8);
    assert_eq!(report["application"]["camera_projection"], "orthographic");
    assert_eq!(report["application"]["selected"], serde_json::json!([id]));
    assert_eq!(report["application"]["dirty"], false);
    assert_eq!(report["application"]["history"], 0);
    assert!(report["application"]["error"].is_null());
    assert!(
        report["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| frame["draw_count"] == 5)
    );
    assert_eq!(fs::read(source)?, bytes);
    Ok(())
}
