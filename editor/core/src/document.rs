use anyhow::{Context, Result, ensure};
use comet_core::{EntityId, InstanceId, atomic_write};
use comet_scene::{
    ContentSource, FieldOverride, PrefabResolver, Scene, SceneSnapshot,
    schema::{FieldKey, FieldValue},
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::Arc,
};

pub const HISTORY_CAPACITY: usize = 256;
#[derive(Clone)]
struct State {
    id: u64,
    source: Arc<ContentSource>,
}
/// Content comparisons occur only on mutation. Queries share immutable values and stable state IDs.
pub struct Document {
    scene: Scene,
    current: State,
    saved: State,
    undo: VecDeque<State>,
    redo: Vec<State>,
    preview: Option<State>,
    next_state: u64,
    revision: u64,
}
impl Document {
    pub fn open(snapshot: SceneSnapshot) -> Result<Self> {
        Self::open_source(ContentSource {
            base: snapshot,
            ..Default::default()
        })
    }
    pub fn open_source(mut source: ContentSource) -> Result<Self> {
        source.validate()?;
        source.canonicalize();
        let scene = Scene::open(source.base.clone())?;
        let current = State {
            id: 0,
            source: Arc::new(source),
        };
        Ok(Self {
            scene,
            saved: current.clone(),
            current,
            undo: VecDeque::new(),
            redo: vec![],
            preview: None,
            next_state: 1,
            revision: 0,
        })
    }
    pub fn load(path: &Path) -> Result<Self> {
        Self::open_source(ContentSource::from_json(
            &std::fs::read(path).with_context(|| format!("read {}", path.display()))?,
        )?)
    }
    pub fn snapshot(&mut self) -> SceneSnapshot {
        self.current.source.base.clone()
    }
    pub fn shared_snapshot(&mut self) -> Arc<SceneSnapshot> {
        self.scene.shared_snapshot()
    }
    pub fn world_id(&self) -> comet_core::WorldId {
        self.scene.world_id()
    }
    pub fn source(&self) -> ContentSource {
        self.current.source.as_ref().clone()
    }
    pub fn shared_source(&self) -> Arc<ContentSource> {
        self.current.source.clone()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn state_id(&self) -> u64 {
        self.current.id
    }
    pub fn is_dirty(&self) -> bool {
        self.current.id != self.saved.id
    }
    pub fn history_len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_editing(&self) -> bool {
        self.preview.is_some()
    }
    pub fn begin(&mut self) -> Result<()> {
        ensure!(self.preview.is_none(), "transaction already active");
        self.preview = Some(self.current.clone());
        Ok(())
    }
    fn restore(&mut self, state: State) -> Result<()> {
        if self.current.source == state.source {
            self.current = state;
            return Ok(());
        }
        let revision = self
            .revision
            .checked_add(1)
            .context("document revision exhausted")?;
        self.scene.replace(state.source.base.clone())?;
        self.current = state;
        self.revision = revision;
        Ok(())
    }
    fn apply(&mut self, mut source: ContentSource) -> Result<()> {
        source.validate()?;
        source.canonicalize();
        if *self.current.source == source {
            return Ok(());
        }
        let id = if *self.saved.source == source {
            self.saved.id
        } else if let Some(before) = &self.preview
            && *before.source == source
        {
            before.id
        } else {
            let id = self.next_state;
            self.next_state = self
                .next_state
                .checked_add(1)
                .context("document state exhausted")?;
            id
        };
        self.restore(State {
            id,
            source: Arc::new(source),
        })
    }
    pub fn preview_field(
        &mut self,
        selection: &[EntityId],
        key: FieldKey,
        value: FieldValue,
    ) -> Result<()> {
        ensure!(self.preview.is_some(), "no edit transaction");
        let mut next = self.source();
        next.base.set_field(selection, key, value)?;
        self.apply(next)
    }
    pub fn preview_override(
        &mut self,
        instance: InstanceId,
        value: FieldOverride,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        ensure!(self.preview.is_some(), "no edit transaction");
        let mut next = self.source();
        let target = next
            .instances
            .iter_mut()
            .find(|i| i.id == instance)
            .context("stale prefab selection")?;
        target
            .overrides
            .retain(|o| o.entity != value.entity || o.field != value.field);
        target.overrides.push(value);
        next.canonicalize();
        next.expand(resolver)?;
        self.apply(next)
    }
    fn record(&mut self, before: State) {
        self.undo.push_back(before);
        self.redo.clear();
        while self.undo.len() > HISTORY_CAPACITY {
            self.undo.pop_front();
        }
    }
    pub fn commit(&mut self) -> Result<()> {
        let before = self.preview.take().context("no edit transaction")?;
        if before.source != self.current.source {
            self.record(before);
        }
        Ok(())
    }
    pub fn cancel(&mut self) -> Result<()> {
        let before = self
            .preview
            .as_ref()
            .context("no edit transaction")?
            .clone();
        self.restore(before)?;
        self.preview = None;
        Ok(())
    }
    pub fn undo(&mut self) -> Result<()> {
        ensure!(self.preview.is_none(), "transaction active");
        let before = self.undo.back().context("nothing to undo")?.clone();
        let now = self.current.clone();
        self.restore(before)?;
        self.undo.pop_back();
        self.redo.push(now);
        Ok(())
    }
    pub fn redo(&mut self) -> Result<()> {
        ensure!(self.preview.is_none(), "transaction active");
        let after = self.redo.last().context("nothing to redo")?.clone();
        let now = self.current.clone();
        self.restore(after)?;
        self.redo.pop();
        self.undo.push_back(now);
        Ok(())
    }
    pub fn clone_subtrees(&mut self, roots: &[EntityId]) -> Result<BTreeMap<EntityId, EntityId>> {
        ensure!(self.preview.is_none(), "transaction active");
        let before = self.current.clone();
        let mut next = self.source();
        let map = next.base.clone_subtrees(roots)?;
        let copies: Vec<_> = next
            .instances
            .iter()
            .filter_map(|i| {
                i.parent.and_then(|p| map.get(&p)).map(|parent| {
                    let mut copy = i.clone();
                    copy.id = InstanceId::new();
                    copy.parent = Some(*parent);
                    copy
                })
            })
            .collect();
        next.instances.extend(copies);
        if !map.is_empty() {
            self.apply(next)?;
            self.record(before);
        }
        Ok(map)
    }
    /// The savepoint changes only after a complete authoring file is published.
    pub fn save(&mut self, path: &Path) -> Result<()> {
        ensure!(self.preview.is_none(), "cannot save an active transaction");
        atomic_write(path, &self.current.source.to_json()?)?;
        self.saved = self.current.clone();
        Ok(())
    }
}
