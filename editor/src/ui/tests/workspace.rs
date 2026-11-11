use super::*;
use crate::ui::workspace::Pane;
use comet_render::{FrameFeedback, PickFeedback, ViewToken};
use std::sync::Arc;

struct WorkspaceCase {
    _temp: tempfile::TempDir,
    editor: Editor,
    assets: AssetStore,
    ctx: egui::Context,
    number: u64,
}
impl WorkspaceCase {
    fn new() -> Result<Self> {
        let (temp, mut editor, mut assets) = fixture()?;
        let id = editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .find(|e| e.name == "Amber cube")
            .unwrap()
            .id;
        editor.selection.replace([id]);
        let ctx = egui::Context::default();
        comet_application::configure_ui(&ctx);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.style_mut_of(theme, |style| style.animation_time = 0.);
        }
        let mut case = Self {
            _temp: temp,
            editor,
            assets,
            ctx,
            number: 0,
        };
        for _ in 0..3 {
            case.frame(vec![]);
        }
        Ok(case)
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> (ViewRequest, egui::FullOutput) {
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
    fn click(&mut self, pos: Pos2) -> (ViewRequest, egui::FullOutput) {
        self.frame(vec![egui::Event::PointerMoved(pos), button(pos, true)]);
        self.frame(vec![button(pos, false)])
    }
    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
        self.frame(vec![egui::Event::PointerMoved(from.lerp(to, 0.2))]);
        self.frame(vec![egui::Event::PointerMoved(to)]);
        self.frame(vec![button(to, false)]);
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }
    fn rect(&self, pane: Pane) -> Rect {
        self.editor
            .workspace
            .tree
            .tiles
            .rect(self.editor.workspace.pane_id(pane).unwrap())
            .unwrap()
    }
    fn tab_rect(&self, pane: Pane) -> Rect {
        let id = self.editor.workspace.pane_id(pane).unwrap();
        let tabs = self.editor.workspace.tree.tiles.iter().find_map(|(&tile, value)| matches!(value, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) if tabs.children.contains(&id)).then_some(tile)).unwrap();
        self.editor.workspace.tree.tiles.rect(tabs).unwrap()
    }
    fn press(&mut self, text: &str) {
        let (_, output) = self.frame(vec![]);
        let pos = position(&output, text);
        self.click(pos);
    }
}
fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    }
}
fn position(output: &egui::FullOutput, label: &str) -> Pos2 {
    fn find(shape: &egui::epaint::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().rev().find_map(|shape| find(shape, label))
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| find(&shape.shape, label))
        .unwrap_or_else(|| panic!("missing {label}"))
}
fn resize(case: &mut WorkspaceCase) {
    let left = case.rect(Pane::Hierarchy);
    let scene = case.rect(Pane::Scene);
    let right = case.rect(Pane::Inspector);
    let from = Pos2::new((left.right() + scene.left()) * 0.5, 400.);
    case.drag(from, from + Vec2::new(60., 0.));
    assert!(case.rect(Pane::Hierarchy).width() > left.width() + 40.);
    let from = Pos2::new(
        (case.rect(Pane::Inspector).left() + case.rect(Pane::Scene).right()) * 0.5,
        400.,
    );
    case.drag(from, from - Vec2::new(80., 0.));
    assert!(case.rect(Pane::Inspector).width() > right.width() + 50.);
    let project = case.rect(Pane::Project);
    let from = Pos2::new(
        case.rect(Pane::Scene).center().x,
        (case.tab_rect(Pane::Project).top() + case.rect(Pane::Scene).bottom()) * 0.5,
    );
    case.drag(from, from - Vec2::new(0., 80.));
    assert!(case.rect(Pane::Project).height() > project.height() + 50.);
}

#[test]
fn panel_drags_resize_and_redock_without_editing_and_hidden_scene_rejects_stale_picks() -> Result<()>
{
    let mut c = WorkspaceCase::new()?;
    let authoring = c.editor.document.shared_source();
    let entity_count = c.editor.snapshot(&mut c.assets)?.entities.len();
    let history = c.editor.document.history_len();
    let selected = c.editor.selection.ids();
    let initial = c.rect(Pane::Hierarchy);
    let view = c.frame(vec![]).0;
    let stale = c.click(view.viewport.lerp_inside(egui::vec2(0.85, 0.25))).0;
    assert!(stale.pick.is_some());
    let requested_at = c.number - 1;
    resize(&mut c);
    let token = ViewToken {
        world: stale.world,
        view: 1,
        target_generation: 1,
    };
    let other = c
        .editor
        .last_snapshot
        .entities
        .iter()
        .find(|e| !selected.contains(&e.id))
        .unwrap()
        .id;
    c.editor.feedback(&FrameFeedback {
        number: requested_at,
        token,
        presented: true,
        backend: "test".into(),
        worker_thread: "test".into(),
        draw_count: 0,
        gpu_ms: 0.,
        pick: Some(PickFeedback {
            token,
            entities: vec![other],
        }),
    });
    assert_eq!(c.editor.selection.ids(), selected);
    let (_, output) = c.frame(vec![]);
    let log = position(&output, "日志");
    let destination = c.rect(Pane::Inspector).center();
    c.drag(log, destination);
    let log_id = c.editor.workspace.pane_id(Pane::Log).unwrap();
    let inspector_id = c.editor.workspace.pane_id(Pane::Inspector).unwrap();
    assert!(c.editor.workspace.tree.tiles.iter().any(|(_, tile)| matches!(tile, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) if tabs.children.contains(&log_id) && tabs.children.contains(&inspector_id))));
    c.press("视图");
    c.press("场景");
    c.click(Pos2::new(1230., 20.));
    let hidden = c.frame(vec![]).0;
    assert!(
        hidden.snapshot.entities.is_empty() && hidden.selected.is_empty() && hidden.pick.is_none()
    );
    assert_eq!(
        c.editor.snapshot(&mut c.assets)?.entities.len(),
        entity_count
    );
    c.press("视图");
    c.press("场景");
    c.click(Pos2::new(1230., 20.));
    assert!(!c.frame(vec![]).0.snapshot.entities.is_empty());
    c.press("视图");
    c.press("恢复默认布局");
    for _ in 0..3 {
        c.frame(vec![]);
    }
    assert!((c.rect(Pane::Hierarchy).width() - initial.width()).abs() < 1.);
    assert!(Arc::ptr_eq(&authoring, &c.editor.document.shared_source()));
    assert_eq!(c.editor.document.history_len(), history);
    c.editor.begin_translation(&mut c.assets)?;
    c.editor
        .preview_translation(comet_core::math::Vec3::X, &mut c.assets)?;
    let preview = c.editor.document.shared_source();
    c.press("视图");
    c.press("属性");
    c.click(Pos2::new(1230., 20.));
    assert!(!c.editor.document.is_editing());
    assert_eq!(c.editor.document.shared_source(), preview);
    assert_eq!(c.editor.document.history_len(), history + 1);
    c.editor.document.undo()?;
    assert_eq!(c.editor.document.shared_source(), authoring);
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_resized_workspace_draws_the_scene_and_preserves_authoring() -> Result<()> {
    let mut c = WorkspaceCase::new()?;
    resize(&mut c);
    let bytes = fs::read(&c.editor.source)?;
    let source = c.editor.source.clone();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    let mut options = comet_application::HostOptions::new("Comet · 编辑器布局");
    options.headless = true;
    options.smoke = true;
    options.capture = Some(out.join("editor-workspace.png"));
    options.report = Some(out.join("editor-workspace.json"));
    comet_application::run(options, c.assets, c.editor)?;
    assert_eq!(fs::read(source)?, bytes);
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("editor-workspace.json"))?)?;
    assert_eq!(report["application"]["dirty"], false);
    assert_eq!(report["application"]["history"], 0);
    assert!(report["application"]["error"].is_null());
    assert_eq!(report["submitted_frames"], 8);
    assert!(
        report["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| frame["draw_count"] == 5)
    );
    Ok(())
}
