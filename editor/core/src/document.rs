use anyhow::{Context, Result, ensure};
use comet_core::{AssetId, EntityId, InstanceId, atomic_write};
use comet_scene::{
    Character, ContentSource, EntityData, FieldOverride, PhysicsBody, PrefabInstance,
    PrefabResolver, Scene, SceneSnapshot,
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
    /// Current content first, then distinct states retained by savepoint, preview and history.
    pub fn retained_sources(&self) -> impl Iterator<Item = Arc<ContentSource>> + '_ {
        let mut seen = std::collections::BTreeSet::new();
        std::iter::once(&self.current)
            .chain(std::iter::once(&self.saved))
            .chain(self.preview.iter())
            .chain(self.undo.iter())
            .chain(self.redo.iter())
            .filter(move |state| seen.insert(state.id))
            .map(|state| state.source.clone())
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
    /// Body and controller form one candidate so intermediate invalid combinations never publish.
    pub fn preview_physics(
        &mut self,
        id: EntityId,
        body: Option<PhysicsBody>,
        character: Option<Character>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        ensure!(self.preview.is_some(), "no edit transaction");
        let mut next = self.source();
        let entity = next
            .base
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .context("physics component editing requires an authored entity")?;
        entity.physics = body;
        entity.character = character;
        if entity.physics.is_some() || entity.character.is_some() {
            next.base.version = comet_scene::scene::SCENE_VERSION;
        }
        next.expand(resolver)?;
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
    /// Structural commands validate the expanded candidate before publishing one history state.
    fn change<T>(
        &mut self,
        resolver: &mut impl PrefabResolver,
        edit: impl FnOnce(&mut ContentSource) -> Result<T>,
    ) -> Result<T> {
        ensure!(self.preview.is_none(), "transaction active");
        let before = self.current.clone();
        let mut next = self.source();
        let result = edit(&mut next)?;
        next.expand(resolver)?;
        self.apply(next)?;
        if before.source != self.current.source {
            self.record(before);
        }
        Ok(result)
    }
    pub fn insert_entity(
        &mut self,
        entity: EntityData,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.change(resolver, |next| {
            next.base.entities.push(entity);
            Ok(())
        })
    }
    /// Keep the instance declaration below an authored container, including multi-root models.
    pub fn insert_prefab(
        &mut self,
        container: EntityData,
        prefab: AssetId,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.change(resolver, |next| {
            let parent = container.id;
            next.base.entities.push(container);
            next.instances.push(PrefabInstance {
                id: InstanceId::new(),
                prefab,
                parent: Some(parent),
                overrides: vec![],
            });
            Ok(())
        })
    }
    pub fn remove_subtrees(
        &mut self,
        roots: &[EntityId],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.change(resolver, |next| {
            let removed = next.base.remove_subtrees(roots)?;
            next.instances
                .retain(|instance| !instance.parent.is_some_and(|id| removed.contains(&id)));
            Ok(())
        })
    }
    pub fn clone_subtrees(
        &mut self,
        roots: &[EntityId],
        resolver: &mut impl PrefabResolver,
    ) -> Result<BTreeMap<EntityId, EntityId>> {
        self.change(resolver, |next| {
            let map = next.base.clone_subtrees(roots)?;
            let copies: Vec<_> = next
                .instances
                .iter()
                .filter_map(|instance| {
                    instance
                        .parent
                        .and_then(|parent| map.get(&parent))
                        .map(|parent| {
                            let mut copy = instance.clone();
                            copy.id = InstanceId::new();
                            copy.parent = Some(*parent);
                            copy
                        })
                })
                .collect();
            next.instances.extend(copies);
            Ok(map)
        })
    }
    pub fn duplicate_instance(
        &mut self,
        id: InstanceId,
        resolver: &mut impl PrefabResolver,
    ) -> Result<InstanceId> {
        self.change(resolver, |next| {
            let mut copy = next
                .instances
                .iter()
                .find(|instance| instance.id == id)
                .context("stale prefab instance")?
                .clone();
            copy.id = InstanceId::new();
            let id = copy.id;
            next.instances.push(copy);
            Ok(id)
        })
    }
    pub fn remove_instance(
        &mut self,
        id: InstanceId,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.change(resolver, |next| {
            let count = next.instances.len();
            next.instances.retain(|instance| instance.id != id);
            ensure!(next.instances.len() != count, "stale prefab instance");
            Ok(())
        })
    }
    /// The savepoint changes only after a complete authoring file is published.
    pub fn save(&mut self, path: &Path) -> Result<()> {
        ensure!(self.preview.is_none(), "cannot save an active transaction");
        atomic_write(path, &self.current.source.to_json()?)?;
        self.saved = self.current.clone();
        Ok(())
    }
}
