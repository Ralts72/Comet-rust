use comet_core::EntityId;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectMode {
    Replace,
    Toggle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    ids: BTreeSet<EntityId>,
    active: Option<EntityId>,
    anchor: Option<EntityId>,
    // Direct UI intent invalidates in-flight picks, including repeated clicks on the same object.
    generation: u64,
}
impl Selection {
    pub fn active(&self) -> Option<EntityId> {
        self.active
    }
    pub fn ids(&self) -> Vec<EntityId> {
        self.ids.iter().copied().collect()
    }
    pub fn contains(&self, id: EntityId) -> bool {
        self.ids.contains(&id)
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    fn invalidate(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("selection generation exhausted");
    }
    pub fn clear(&mut self) {
        self.replace([]);
    }
    pub fn replace(&mut self, ids: impl IntoIterator<Item = EntityId>) {
        self.invalidate();
        self.ids.clear();
        self.active = None;
        for id in ids {
            self.ids.insert(id);
            self.active = Some(id);
        }
        self.anchor = self.active;
    }
    pub fn choose(&mut self, id: Option<EntityId>, mode: SelectMode) {
        self.invalidate();
        self.apply_pick(id, mode);
    }
    pub fn activate(&mut self, id: EntityId) {
        if self.ids.contains(&id) {
            self.invalidate();
            self.active = Some(id);
            self.anchor = Some(id);
        } else {
            self.choose(Some(id), SelectMode::Replace);
        }
    }
    /// Pick feedback is ordered by the render worker; it must not invalidate later clicks in flight.
    pub fn apply_pick(&mut self, ids: impl IntoIterator<Item = EntityId>, mode: SelectMode) {
        let ids: BTreeSet<_> = ids.into_iter().collect();
        if mode == SelectMode::Toggle && ids.is_empty() {
            return;
        }
        match mode {
            SelectMode::Replace => {
                self.ids = ids;
                self.active = self.ids.last().copied();
            }
            SelectMode::Toggle => {
                for id in ids {
                    if self.ids.remove(&id) {
                        if self.active == Some(id) {
                            self.active = self.ids.last().copied();
                        }
                    } else {
                        self.ids.insert(id);
                        self.active = Some(id);
                    }
                }
            }
        }
        self.anchor = self.active;
    }
    pub fn range(&mut self, visible: &[EntityId], target: EntityId, additive: bool) {
        let anchor = self.anchor.or(self.active).unwrap_or(target);
        let start = visible.iter().position(|id| *id == anchor);
        let end = visible.iter().position(|id| *id == target);
        let (Some(start), Some(end)) = (start, end) else {
            self.choose(Some(target), SelectMode::Replace);
            return;
        };
        self.invalidate();
        if !additive {
            self.ids.clear();
        }
        self.ids.extend(&visible[start.min(end)..=start.max(end)]);
        self.active = Some(target);
        self.anchor = Some(anchor);
    }
    pub fn retain(&mut self, mut keep: impl FnMut(EntityId) -> bool) {
        let before = self.ids.len();
        self.ids.retain(|id| keep(*id));
        if self.ids.len() != before {
            self.invalidate();
            if self.active.is_some_and(|id| !self.ids.contains(&id)) {
                self.active = self.ids.last().copied();
            }
            if self.anchor.is_some_and(|id| !self.ids.contains(&id)) {
                self.anchor = self.active;
            }
        }
    }
}

#[cfg(test)]
mod tests;

pub(crate) struct SelectionPick {
    pub request: comet_render::PickRequest,
    pub mode: SelectMode,
    pub selection_generation: u64,
}
