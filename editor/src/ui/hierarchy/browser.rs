use crate::selection::Selection;
use comet_core::{EntityId, WorldId};
use comet_scene::{EntityData, SceneSnapshot};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) struct Index {
    snapshot: Arc<SceneSnapshot>,
    entities: BTreeMap<EntityId, usize>,
    children: BTreeMap<Option<EntityId>, Vec<EntityId>>,
}

#[cfg(test)]
mod tests;
impl Index {
    fn new(snapshot: Arc<SceneSnapshot>) -> Self {
        let mut entities = BTreeMap::new();
        let mut children: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (index, entity) in snapshot.entities.iter().enumerate() {
            entities.insert(entity.id, index);
            children.entry(entity.parent).or_default().push(entity.id);
        }
        Self {
            snapshot,
            entities,
            children,
        }
    }
    pub fn entity(&self, id: EntityId) -> &EntityData {
        &self.snapshot.entities[self.entities[&id]]
    }

    fn filtered(&self, query: &str) -> (Option<BTreeSet<EntityId>>, usize) {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return (None, self.entities.len());
        }
        let mut included = BTreeSet::new();
        let mut matches = 0;
        for entity in &self.snapshot.entities {
            if entity.name.to_lowercase().contains(&query) || entity.id.to_string().contains(&query)
            {
                matches += 1;
                let mut next = Some(entity.id);
                while let Some(id) = next {
                    if !included.insert(id) {
                        break;
                    }
                    next = self.entity(id).parent;
                }
            }
        }
        (Some(included), matches)
    }
}

#[derive(Clone)]
pub(super) struct Row {
    pub id: EntityId,
    pub depth: usize,
    pub branch: bool,
    pub open: bool,
}
#[derive(Clone, Default)]
pub(super) struct View {
    pub rows: Vec<Row>,
    pub ids: Vec<EntityId>,
    matches: usize,
    visible: BTreeSet<EntityId>,
    pub depth: usize,
}

#[derive(Clone)]
pub(super) struct Browser {
    world: WorldId,
    pub index: Arc<Index>,
    query: String,
    closed: BTreeSet<EntityId>,
    observed: (u64, Option<EntityId>),
    pub view: Arc<View>,
    dirty: bool,
    reset_scroll: bool,
    reveal: Option<EntityId>,
}

fn state_id() -> egui::Id {
    egui::Id::unique("hierarchy-browser")
}
impl Browser {
    pub fn load(
        ctx: &egui::Context,
        world: WorldId,
        snapshot: Arc<SceneSnapshot>,
        selection: &Selection,
    ) -> Self {
        let mut browser = ctx
            .data_mut(|data| data.remove_temp::<Option<Self>>(state_id()))
            .flatten()
            .filter(|browser| browser.world == world)
            .unwrap_or_else(|| Self {
                world,
                index: Arc::new(Index::new(snapshot.clone())),
                query: String::new(),
                closed: BTreeSet::new(),
                observed: (u64::MAX, None),
                view: Arc::default(),
                dirty: true,
                reset_scroll: true,
                reveal: None,
            });
        if !Arc::ptr_eq(&browser.index.snapshot, &snapshot) {
            browser.index = Arc::new(Index::new(snapshot));
            browser
                .closed
                .retain(|id| browser.index.entities.contains_key(id));
            browser.dirty = true;
        }
        let observed = (selection.generation(), selection.active());
        if browser.observed != observed {
            browser.observed = observed;
            browser.reveal_selection(selection.active());
        }
        browser
    }
    pub fn store(self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(state_id(), Some(self)));
    }

    fn reveal_selection(&mut self, active: Option<EntityId>) {
        let Some(id) = active.filter(|id| self.index.entities.contains_key(id)) else {
            return;
        };
        if let Some(included) = self.index.filtered(&self.query).0 {
            if included.contains(&id) {
                // Filter expansion is temporary, including selections made within its results.
                self.reveal = Some(id);
                return;
            }
            self.query.clear();
            self.dirty = true;
        }
        let mut parent = self.index.entity(id).parent;
        while let Some(id) = parent {
            self.dirty |= self.closed.remove(&id);
            parent = self.index.entity(id).parent;
        }
        self.reveal = Some(id);
    }
    pub fn toggle(&mut self, id: EntityId) {
        if !self.query.trim().is_empty() {
            return;
        }
        if !self.closed.remove(&id) {
            self.closed.insert(id);
        }
        self.dirty = true;
    }
    pub fn controls(&mut self, ui: &mut egui::Ui, selection: &Selection) {
        let search = ui.make_persistent_id("hierarchy-search");
        // egui clears focus for Escape before widgets receive the current frame.
        let escape = ui
            .memory(|memory| memory.has_focus(search) || memory.had_focus_last_frame(search))
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 42.).max(50.);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(search)
                    .hint_text("搜索实体...")
                    .desired_width(width),
            );
            self.dirty |= response.changed();
            self.reset_scroll |= response.changed();
            if escape
                || ui
                    .add_enabled(!self.query.is_empty(), egui::Button::new("清除"))
                    .clicked()
            {
                self.query.clear();
                self.dirty = true;
                self.reset_scroll = true;
                response.surrender_focus();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("全部展开").clicked() {
                self.closed.clear();
                self.dirty = true;
            }
            if ui.small_button("全部折叠").clicked() {
                self.closed = self.index.children.keys().flatten().copied().collect();
                self.dirty = true;
            }
            if ui
                .add_enabled(
                    selection.active().is_some(),
                    egui::Button::new("定位选中").small(),
                )
                .clicked()
            {
                self.reveal_selection(selection.active());
            }
        });
        if self.dirty {
            let (included, matches) = self.index.filtered(&self.query);
            let mut view = View {
                matches,
                ..Default::default()
            };
            let mut pending: Vec<_> = self
                .index
                .children
                .get(&None)
                .into_iter()
                .flatten()
                .rev()
                .map(|id| (*id, 0))
                .collect();
            while let Some((id, depth)) = pending.pop() {
                if included
                    .as_ref()
                    .is_some_and(|included| !included.contains(&id))
                {
                    continue;
                }
                let children = self.index.children.get(&Some(id));
                let open = included.is_some() || !self.closed.contains(&id);
                view.rows.push(Row {
                    id,
                    depth,
                    branch: children.is_some(),
                    open,
                });
                view.ids.push(id);
                view.visible.insert(id);
                view.depth = view.depth.max(depth);
                if open {
                    pending.extend(
                        children
                            .into_iter()
                            .flatten()
                            .rev()
                            .map(|child| (*child, depth + 1)),
                    );
                }
            }
            self.view = Arc::new(view);
            self.dirty = false;
        }
        let hidden = selection
            .ids()
            .into_iter()
            .filter(|id| !self.view.visible.contains(id))
            .count();
        ui.small(format!(
            "显示 {} / {} · 已选 {}{}",
            self.view.ids.len(),
            self.index.entities.len(),
            selection.len(),
            if hidden == 0 {
                String::new()
            } else {
                format!("（隐藏 {hidden}）")
            }
        ));
        if !self.query.trim().is_empty() {
            ui.weak(format!("匹配 {} 项，展开祖先路径", self.view.matches));
        }
    }
    pub fn filtered(&self) -> bool {
        !self.query.trim().is_empty()
    }
    pub fn scroll(&mut self, ui: &egui::Ui, row_height: f32) -> Option<egui::Vec2> {
        let reset = std::mem::take(&mut self.reset_scroll);
        let Some(target) = self.reveal.take() else {
            return reset.then_some(egui::Vec2::ZERO);
        };
        let index = self.view.ids.iter().position(|id| *id == target)?;
        let id = ui.make_persistent_id(("hierarchy-rows", self.world));
        let mut offset = if reset {
            egui::Vec2::ZERO
        } else {
            egui::scroll_area::State::load(ui.ctx(), id)
                .unwrap_or_default()
                .clamped_offset()
        };
        let before = offset;
        let top = index as f32 * (row_height + ui.spacing().item_spacing.y);
        let bottom = top + row_height;
        let height = (ui.available_height() - row_height).max(row_height);
        if top < offset.y {
            offset.y = top;
        }
        if bottom > offset.y + height {
            offset.y = (bottom - height).max(0.);
        }
        let left = self.view.rows[index].depth as f32 * ui.spacing().indent;
        let right = left + ui.available_width().min(164.);
        if left < offset.x {
            offset.x = left;
        }
        if right > offset.x + ui.available_width() {
            offset.x = (right - ui.available_width()).max(0.);
        }
        (reset || offset != before).then_some(offset)
    }
}
