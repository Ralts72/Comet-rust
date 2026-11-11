//! Editor panel placement owns no document, asset or simulation state.
use super::{hierarchy, properties, resources, viewport};
use crate::session::Editor;
use comet_asset::AssetStore;
use egui_tiles::{Behavior, Container, EditAction, Linear, LinearDir, Tile, TileId, Tiles, Tree};
use serde::{Deserialize, Serialize};

mod storage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) enum Pane {
    Hierarchy,
    Scene,
    Inspector,
    Project,
    Log,
}
impl Pane {
    const ALL: [Self; 5] = [
        Self::Hierarchy,
        Self::Scene,
        Self::Inspector,
        Self::Project,
        Self::Log,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Hierarchy => "层级",
            Self::Scene => "场景",
            Self::Inspector => "属性",
            Self::Project => "项目",
            Self::Log => "日志",
        }
    }
}

pub(crate) struct Workspace {
    pub(super) tree: Tree<Pane>,
    storage: storage::Storage,
    dirty: bool,
    cancel_input: bool,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            tree: default_tree(),
            storage: Default::default(),
            dirty: false,
            cancel_input: false,
        }
    }
}
impl Workspace {
    pub(crate) fn enable_user_state(&mut self) {
        self.storage.enable(&mut self.tree);
    }
    pub(crate) fn flush(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.storage.save(&self.tree);
        }
    }
    pub(super) fn focus(&mut self, pane: Pane) {
        if let Some(id) = self.pane_id(pane) {
            self.tree.set_visible(id, true);
            self.tree
                .make_active(|_, tile| matches!(tile, Tile::Pane(value) if *value == pane));
            self.dirty = true;
            self.cancel_input = true;
        }
    }
    pub(super) fn menu(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        ui.menu_button("视图", |ui| {
            ui.weak("拖动标签停靠，拖动分隔线调整宽高");
            for pane in Pane::ALL {
                if let Some(id) = self.pane_id(pane) {
                    let mut visible = self.tree.tiles.is_visible(id);
                    if ui.checkbox(&mut visible, pane.title()).changed() {
                        if let Err(error) = editor.commit() {
                            editor.notice(Err(error));
                            continue;
                        }
                        self.tree.set_visible(id, visible);
                        if visible {
                            self.tree.make_active(
                                |_, tile| matches!(tile, Tile::Pane(value) if *value == pane),
                            );
                        }
                        self.dirty = true;
                        self.cancel_input = true;
                        ui.ctx().stop_dragging();
                    }
                }
            }
            ui.separator();
            if ui.button("恢复默认布局").clicked() {
                if let Err(error) = editor.commit() {
                    editor.notice(Err(error));
                    return;
                }
                self.tree = default_tree();
                self.storage.allow_save();
                self.dirty = true;
                self.cancel_input = true;
                ui.ctx().stop_dragging();
                ui.close();
            }
            self.storage.warning(ui);
        });
    }
    pub(super) fn pane_id(&self, pane: Pane) -> Option<TileId> {
        self.tree.tiles.iter().find_map(|(&id, tile)| {
            matches!(tile, Tile::Pane(value) if *value == pane).then_some(id)
        })
    }
    pub(super) fn show(
        &mut self,
        editor: &mut Editor,
        root: &mut egui::Ui,
        assets: &mut AssetStore,
        active: bool,
    ) -> viewport::ViewportResponse {
        let mut view = None;
        let mut edited = false;
        let mut layout_changed = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                let background = ui.painter().add(egui::Shape::Noop);
                self.tree.ui(
                    &mut Panels {
                        editor,
                        assets,
                        active,
                        view: &mut view,
                        edited: &mut edited,
                        layout_changed: &mut layout_changed,
                        user_selection: ui.input(|input| {
                            input.pointer.any_click() || input.key_pressed(egui::Key::Tab)
                        }),
                    },
                    ui,
                );
                if view.as_ref().is_none_or(|view| !view.is_visible()) {
                    ui.painter().set(
                        background,
                        egui::Shape::rect_filled(ui.max_rect(), 0., ui.visuals().panel_fill),
                    );
                }
            });
        layout_changed |= view.as_ref().is_none_or(|view| !view.is_visible());
        let mut view = view.unwrap_or_else(|| viewport::ViewportResponse::hidden(root));
        self.dirty |= edited;
        if layout_changed || self.cancel_input {
            editor.picking.clear();
            view.cancel_input(root.ctx());
            self.cancel_input = false;
        }
        if !root.input(|input| input.pointer.any_down()) {
            self.flush();
        }
        view
    }
}

fn default_tree() -> Tree<Pane> {
    let mut tiles = Tiles::default();
    let [hierarchy, scene, inspector, project, log] = Pane::ALL.map(|pane| tiles.insert_pane(pane));
    let left = tiles.insert_tab_tile(vec![hierarchy]);
    let view = tiles.insert_tab_tile(vec![scene]);
    let right = tiles.insert_tab_tile(vec![inspector]);
    let bottom = tiles.insert_tab_tile(vec![project, log]);
    let mut center = Linear::new(LinearDir::Vertical, vec![view, bottom]);
    center.shares[view] = 609.;
    center.shares[bottom] = 120.;
    let center = tiles.insert_container(center);
    let mut row = Linear::new(LinearDir::Horizontal, vec![left, center, right]);
    row.shares[left] = 220.;
    row.shares[center] = 780.;
    row.shares[right] = 280.;
    let root = tiles.insert_container(row);
    Tree::new(egui::Id::unique("editor-workspace"), root, tiles)
}

struct Panels<'a> {
    editor: &'a mut Editor,
    assets: &'a mut AssetStore,
    active: bool,
    view: &'a mut Option<viewport::ViewportResponse>,
    edited: &'a mut bool,
    layout_changed: &'a mut bool,
    user_selection: bool,
}
impl Behavior<Pane> for Panels<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        pane.title().into()
    }
    fn pane_ui(&mut self, ui: &mut egui::Ui, _: TileId, pane: &mut Pane) -> egui_tiles::UiResponse {
        // Pane widget identities survive reparenting between tab and split containers.
        ui.scope_builder(
            egui::UiBuilder::new().scope_id(egui::Id::unique(("editor-pane", *pane))),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                if *pane == Pane::Scene {
                    *self.view = Some(viewport::show(self.editor, ui, self.assets, self.active));
                } else {
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0., ui.visuals().panel_fill);
                    egui::Frame::NONE.inner_margin(6).show(ui, |ui| match pane {
                        Pane::Hierarchy => hierarchy::draw(self.editor, ui, self.assets),
                        Pane::Inspector => properties::draw(self.editor, ui, self.assets),
                        Pane::Project => resources::draw(self.editor, ui, self.assets),
                        Pane::Log => resources::console(self.editor, ui),
                        Pane::Scene => unreachable!(),
                    });
                }
            },
        );
        egui_tiles::UiResponse::None
    }
    fn gap_width(&self, _: &egui::Style) -> f32 {
        5.
    }
    fn min_size(&self) -> f32 {
        96.
    }
    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
    fn is_tile_draggable(&self, _: &Tiles<Pane>, _: TileId) -> bool {
        !self.editor.document.is_editing() && self.editor.drag_origin.is_none()
    }
    fn is_tab_closable(&self, tiles: &Tiles<Pane>, id: TileId) -> bool {
        matches!(tiles.get(id), Some(Tile::Pane(_)))
    }
    fn on_tab_close(&mut self, tiles: &mut Tiles<Pane>, id: TileId) -> bool {
        if let Err(error) = self.editor.commit() {
            self.editor.notice(Err(error));
            return false;
        }
        tiles.set_visible(id, false);
        *self.edited = true;
        *self.layout_changed = true;
        false
    }
    fn on_edit(&mut self, action: EditAction) {
        if action == EditAction::TabSelected
            && self.user_selection
            && self.editor.document.is_editing()
        {
            let result = self.editor.commit();
            self.editor.notice(result);
        }
        *self.edited = true;
        *self.layout_changed |= !matches!(action, EditAction::TabSelected);
    }
    fn tab_bg_color(
        &self,
        visuals: &egui::Visuals,
        _: &Tiles<Pane>,
        _: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            egui::Color32::from_rgb(43, 65, 88)
        } else {
            visuals.window_fill
        }
    }
    fn resize_stroke(&self, _: &egui::Style, state: egui_tiles::ResizeState) -> egui::Stroke {
        let color = match state {
            egui_tiles::ResizeState::Idle => egui::Color32::from_gray(42),
            _ => egui::Color32::from_rgb(66, 150, 250),
        };
        egui::Stroke::new(3., color)
    }
}
