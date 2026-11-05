use comet_core::WorldId;
use comet_runtime::ScriptDiagnostic;
use std::collections::{BTreeSet, VecDeque};

const CAPACITY: usize = 128;
const MESSAGE_BYTES: usize = 4096;

#[derive(Default)]
pub(crate) struct ScriptConsole {
    entries: VecDeque<ScriptDiagnostic>,
    seen: BTreeSet<(WorldId, u64)>,
    pub show: bool,
}
impl ScriptConsole {
    pub fn collect(&mut self, diagnostics: &VecDeque<ScriptDiagnostic>) {
        let current: BTreeSet<_> = diagnostics
            .iter()
            .map(|entry| (entry.world, entry.sequence))
            .collect();
        self.seen.retain(|key| current.contains(key));
        for diagnostic in diagnostics {
            if !self.seen.insert((diagnostic.world, diagnostic.sequence)) {
                continue;
            }
            let mut entry = diagnostic.clone();
            if entry.message.len() > MESSAGE_BYTES {
                let mut end = MESSAGE_BYTES;
                while !entry.message.is_char_boundary(end) {
                    end -= 1;
                }
                entry.message.truncate(end);
            }
            self.entries.push_back(entry);
            while self.entries.len() > CAPACITY {
                self.entries.pop_front();
            }
        }
    }
    pub fn entries(&self) -> &VecDeque<ScriptDiagnostic> {
        &self.entries
    }
    /// Clearing visible messages keeps the cursor so the current runtime queue does not replay them.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.entries
                .iter()
                .map(|entry| {
                    serde_json::json!({
                        "world": entry.world, "sequence": entry.sequence, "entity": entry.entity,
                        "asset": entry.asset, "tick": entry.tick, "phase": entry.phase,
                        "message": entry.message, "failed": entry.failed,
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests;
