use super::*;
use anyhow::Result;

fn paint(ctx: &egui::Context, browser: &mut Browser, selection: &Selection) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(220., 600.),
            )),
            ..Default::default()
        },
        |ui| browser.controls(ui, selection),
    );
    output.textures_delta.clear();
}

#[test]
fn browser_caches_only_immutable_projections_and_reveals_changed_selection_across_worlds() {
    let mut root = EntityData::new("root");
    let mut child = EntityData::new("Needle");
    child.parent = Some(root.id);
    let mut leaf = EntityData::new("leaf");
    leaf.parent = Some(child.id);
    root.links.target = Some(comet_scene::EntityRef::local(leaf.id));
    let ids = [root.id, child.id, leaf.id];
    let snapshot = Arc::new(SceneSnapshot {
        entities: vec![root, child, leaf],
        ..Default::default()
    });
    let baseline = snapshot.as_ref().clone();
    let ctx = egui::Context::default();
    let world = WorldId::new();
    let mut selection = Selection::default();
    selection.replace([ids[2]]);
    let mut browser = Browser::load(&ctx, world, snapshot.clone(), &selection);
    paint(&ctx, &mut browser, &selection);
    assert_eq!(browser.view.ids, ids);
    let cached_index = browser.index.clone();
    let cached_view = browser.view.clone();
    browser.store(&ctx);
    let mut browser = Browser::load(&ctx, world, snapshot.clone(), &selection);
    paint(&ctx, &mut browser, &selection);
    assert!(Arc::ptr_eq(&cached_index, &browser.index));
    assert!(Arc::ptr_eq(&cached_view, &browser.view));
    browser.toggle(ids[0]);
    paint(&ctx, &mut browser, &selection);
    assert_eq!(browser.view.ids, [ids[0]]);
    browser.query = "nEeDlE".into();
    browser.dirty = true;
    paint(&ctx, &mut browser, &selection);
    assert_eq!(browser.view.ids, [ids[0], ids[1]]);
    assert_eq!(browser.view.matches, 1);
    assert_eq!(selection.ids(), [ids[2]]);
    browser.query.clear();
    browser.dirty = true;
    paint(&ctx, &mut browser, &selection);
    assert_eq!(browser.view.ids, [ids[0]]);
    browser.query = ids[1].to_string().to_uppercase();
    browser.dirty = true;
    paint(&ctx, &mut browser, &selection);
    assert_eq!(browser.view.ids, [ids[0], ids[1]]);
    browser.store(&ctx);
    selection.choose(Some(ids[2]), crate::selection::SelectMode::Replace);
    let mut browser = Browser::load(&ctx, world, snapshot.clone(), &selection);
    paint(&ctx, &mut browser, &selection);
    assert!(browser.query.is_empty());
    assert_eq!(browser.view.ids, ids);
    browser.query = "stale query".into();
    browser.closed.insert(ids[0]);
    browser.store(&ctx);
    let mut other = Browser::load(&ctx, WorldId::new(), snapshot.clone(), &selection);
    paint(&ctx, &mut other, &selection);
    assert!(other.query.is_empty());
    assert!(other.closed.is_empty());
    assert_eq!(other.view.ids, ids);
    assert_eq!(*snapshot, baseline);
    // A deep but valid hierarchy must not consume the call stack while filtering/projecting.
    let mut entities = Vec::new();
    let mut parent = None;
    for n in 0..2048 {
        let mut entity = EntityData::new(if n == 2047 { "deep target" } else { "branch" });
        entity.parent = parent;
        parent = Some(entity.id);
        entities.push(entity);
    }
    let deep = Arc::new(SceneSnapshot {
        entities,
        ..Default::default()
    });
    let mut browser = Browser::load(&ctx, WorldId::new(), deep, &Selection::default());
    browser.query = "deep target".into();
    paint(&ctx, &mut browser, &Selection::default());
    assert_eq!(browser.view.ids.len(), 2048);
    assert_eq!(browser.view.depth, 2047);
}

struct Capture {
    editor: crate::session::Editor,
    rows: usize,
    matches: usize,
}
impl comet_application::Application for Capture {
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut comet_asset::AssetStore,
        input: &comet_application::FrameContext,
    ) -> Result<comet_application::ViewRequest> {
        if input.number == 0 {
            let snapshot = Arc::new(self.editor.document.expand(assets)?);
            let parent = snapshot
                .entities
                .iter()
                .find(|e| e.name == "定位路径")
                .unwrap()
                .id;
            let mut browser = Browser::load(
                ui.ctx(),
                self.editor.world(),
                snapshot,
                &self.editor.selection,
            );
            browser.query = "匹配目标".into();
            browser.closed.insert(parent);
            browser.dirty = true;
            browser.store(ui.ctx());
        }
        let request = self.editor.frame(ui, assets, input)?;
        let browser = ui
            .ctx()
            .data(|data| data.get_temp::<Option<Browser>>(state_id()))
            .flatten()
            .unwrap();
        self.rows = browser.view.ids.len();
        self.matches = browser.view.matches;
        Ok(request)
    }
    fn feedback(&mut self, feedback: &comet_render::FrameFeedback) {
        self.editor.feedback(feedback);
    }
    fn report(&mut self) -> serde_json::Value {
        let mut report = self.editor.report();
        report["hierarchy_visible_rows"] = self.rows.into();
        report["hierarchy_matches"] = self.matches.into();
        report
    }
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_filtered_hierarchy_draws_selected_path_without_editing_authoring() -> Result<()> {
    let (_temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let model = editor
        .snapshot(&mut assets)?
        .entities
        .iter()
        .find(|e| e.name.contains("Amber") && e.model.is_some())
        .unwrap()
        .model
        .clone();
    let mut content = editor.document.source();
    let parent = EntityData::new("定位路径");
    let mut child = EntityData::new("匹配目标");
    child.parent = Some(parent.id);
    child.model = model;
    child.transform.translation = [0., 1.5, 0.];
    let child_id = child.id;
    content
        .base
        .entities
        .extend([parent, child, EntityData::new("无关作者")]);
    editor.document = comet_editor_core::Document::open_source(content)?;
    editor.selection.replace([child_id]);
    let bytes = std::fs::read(&editor.source)?;
    let source = editor.source.clone();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    let mut options = comet_application::HostOptions::new("Comet · 层级浏览");
    options.headless = true;
    options.smoke = true;
    options.capture = Some(out.join("editor-hierarchy-browser.png"));
    options.report = Some(out.join("editor-hierarchy-browser.json"));
    comet_application::run(
        options,
        assets,
        Capture {
            editor,
            rows: 0,
            matches: 0,
        },
    )?;
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("editor-hierarchy-browser.json"))?)?;
    assert_eq!(report["application"]["hierarchy_visible_rows"], 2);
    assert_eq!(report["application"]["hierarchy_matches"], 1);
    assert_eq!(report["application"]["history"], 0);
    assert_eq!(report["application"]["dirty"], false);
    assert!(report["application"]["error"].is_null());
    assert_eq!(report["submitted_frames"], 8);
    assert!(
        report["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| frame["draw_count"] == 6)
    );
    assert_eq!(std::fs::read(source)?, bytes);
    Ok(())
}
