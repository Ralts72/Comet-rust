//! Selection intent stays on the UI thread; render feedback supplies the matching batch.
use crate::selection::SelectMode;
use comet_core::{EntityId, WorldId};
use comet_render::FrameFeedback;
use std::collections::VecDeque;

// The render worker's bounded queues keep normal traffic below this limit.
const MAX_PENDING_PICKS: usize = 16;

struct PendingPick {
    frame: u64,
    world: WorldId,
    target_generation: u64,
    selection_generation: u64,
    mode: SelectMode,
}

#[derive(Default)]
pub(crate) struct Picking {
    pending: VecDeque<PendingPick>,
}

impl Picking {
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    pub fn request(
        &mut self,
        frame: u64,
        world: WorldId,
        target_generation: u64,
        selection_generation: u64,
        mode: SelectMode,
    ) {
        self.pending.retain(|pick| {
            pick.frame < frame
                && pick.world == world
                && pick.target_generation == target_generation
                && pick.selection_generation == selection_generation
        });
        self.pending.push_back(PendingPick {
            frame,
            world,
            target_generation,
            selection_generation,
            mode,
        });
        while self.pending.len() > MAX_PENDING_PICKS {
            self.pending.pop_front();
        }
    }

    pub fn resolve<'a>(
        &mut self,
        feedback: &'a FrameFeedback,
        world: WorldId,
        target_generation: u64,
        selection_generation: u64,
    ) -> Option<(SelectMode, &'a [EntityId])> {
        // Worker feedback is ordered. A later frame also retires requests whose old
        // target tokens were filtered by the application host.
        let pending = loop {
            if self.pending.front()?.frame > feedback.number {
                return None;
            }
            let pending = self.pending.pop_front()?;
            if pending.frame == feedback.number {
                break pending;
            }
        };
        let pick = feedback.pick.as_ref()?;
        if pick.token != feedback.token
            || pick.token.world != pending.world
            || pick.token.target_generation != pending.target_generation
            || pending.world != world
            || pending.target_generation != target_generation
            || pending.selection_generation != selection_generation
        {
            return None;
        }
        Some((pending.mode, &pick.entities))
    }
}

#[cfg(test)]
mod tests;
