use anyhow::{Context, Result, ensure};
use comet_core::{AssetId, EntityId, InstanceId, atomic_write};
use comet_scene::{
    Character, ContentSource, EntityData, EntityRef, FieldOverride, Model, NavigationAgent,
    NavigationRegion, PhysicsBody, PrefabInstance, PrefabResolver, Scene, SceneSnapshot, Script,
    schema::{FieldKey, FieldValue},
};
use std::{collections::VecDeque, path::Path, sync::Arc};

pub const HISTORY_CAPACITY: usize = 256;
mod hierarchy;
mod joints;
mod selection;
pub use joints::JointEdit;
pub use selection::{CloneMap, FieldEdit, FieldTarget, ObjectTarget, OverrideReset};
#[derive(Clone)]
struct State {
    id: u64,
    source: Arc<ContentSource>,
}
/// Content comparisons occur only on mutation. Queries share immutable values and stable state IDs.
pub struct Document {
    scene: Scene,
    prefab_asset: Option<AssetId>,
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
            prefab_asset: None,
            saved: current.clone(),
            current,
            undo: VecDeque::new(),
            redo: vec![],
            preview: None,
            next_state: 1,
            revision: 0,
        })
    }
    /// A prefab keeps its authoring UUIDs; its root identity is validation context, not file content.
    pub fn open_prefab(
        root: AssetId,
        source: ContentSource,
        resolver: &mut impl PrefabResolver,
    ) -> Result<Self> {
        let mut document = Self::open_source(source)?;
        document.prefab_asset = Some(root);
        document.expand(resolver)?;
        Ok(document)
    }
    pub fn prefab_asset(&self) -> Option<AssetId> {
        self.prefab_asset
    }
    pub fn expand(&self, resolver: &mut impl PrefabResolver) -> Result<SceneSnapshot> {
        self.expand_source(&self.current.source, resolver)
    }
    fn expand_source(
        &self,
        source: &ContentSource,
        resolver: &mut impl PrefabResolver,
    ) -> Result<SceneSnapshot> {
        match self.prefab_asset {
            Some(root) => source.expand_prefab(root, resolver),
            None => source.expand(resolver),
        }
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
        comet_scene::schema::descriptor(key)?;
        next.base.version = next
            .base
            .version
            .max(comet_scene::schema::required_scene_version(key));
        let edits: Vec<_> = selection
            .iter()
            .map(|id| FieldEdit {
                target: FieldTarget::Authored(*id),
                field: key,
                value: value.clone(),
            })
            .collect();
        selection::write_fields(&mut next, &edits)?;
        self.apply(next)
    }
    pub fn preview_name(&mut self, id: EntityId, name: String) -> Result<()> {
        ensure!(self.preview.is_some(), "no edit transaction");
        let mut next = self.source();
        next.base
            .entities
            .iter_mut()
            .find(|entity| entity.id == id)
            .context("renaming requires an authored entity")?
            .name = name;
        self.apply(next)
    }
    fn preview_source(
        &mut self,
        resolver: &mut impl PrefabResolver,
        edit: impl FnOnce(&mut ContentSource) -> Result<()>,
    ) -> Result<()> {
        ensure!(self.preview.is_some(), "no edit transaction");
        let mut next = self.source();
        edit(&mut next)?;
        next.base.version = next
            .base
            .entities
            .iter()
            .map(EntityData::required_scene_version)
            .fold(next.base.version, u32::max);
        self.expand_source(&next, resolver)?;
        self.apply(next)
    }

    fn preview_component(
        &mut self,
        id: EntityId,
        resolver: &mut impl PrefabResolver,
        edit: impl FnOnce(&mut EntityData),
    ) -> Result<()> {
        self.preview_source(resolver, |next| {
            let entity = next
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == id)
                .context("component editing requires an authored entity")?;
            edit(entity);
            Ok(())
        })
    }

    /// Body and controller form one candidate so intermediate invalid combinations never publish.
    pub fn preview_physics(
        &mut self,
        id: EntityId,
        body: Option<PhysicsBody>,
        character: Option<Character>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_component(id, resolver, |entity| {
            entity.physics = body;
            entity.character = character;
        })
    }
    pub fn preview_override(
        &mut self,
        instance: InstanceId,
        value: FieldOverride,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_fields(
            &[FieldEdit {
                target: FieldTarget::Prefab {
                    instance,
                    entity: value.entity,
                },
                field: value.field,
                value: value.value,
            }],
            resolver,
        )
    }
    /// Model structure belongs to authored entities; asset and draw validation belongs to the host.
    pub fn preview_model(
        &mut self,
        id: EntityId,
        model: Option<Model>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_component(id, resolver, |entity| {
            entity.model = model;
            if entity.model.is_none() {
                entity.skin = None;
            }
        })
    }

    pub fn preview_script(
        &mut self,
        id: EntityId,
        script: Option<Script>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_component(id, resolver, |entity| {
            entity.script = script;
        })
    }

    pub fn preview_audio_source(
        &mut self,
        id: EntityId,
        audio: Option<comet_scene::AudioSource>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_component(id, resolver, |entity| {
            entity.audio_source = audio;
        })
    }

    /// Region and agent edits validate the entire candidate before replacing the accepted preview.
    pub fn preview_navigation(
        &mut self,
        id: EntityId,
        region: Option<NavigationRegion>,
        agent: Option<NavigationAgent>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_component(id, resolver, |entity| {
            entity.navigation_region = region;
            entity.navigation_agent = agent;
        })
    }

    pub fn preview_animation(
        &mut self,
        id: EntityId,
        animator: Option<comet_scene::Animator>,
        skin: Option<comet_scene::SkinBinding>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_source(resolver, |next| {
            let entity = next
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == id)
                .context("animation component editing requires an authored entity")?;
            let removed = entity.animator.is_some() && animator.is_none();
            entity.animator = animator;
            entity.skin = skin;
            if removed {
                // The entity still exists, so retire only references to its removed controller component.
                for entity in &mut next.base.entities {
                    if let Some(skin) = &mut entity.skin
                        && skin.animator == Some(EntityRef::local(id))
                    {
                        skin.animator = None;
                    }
                }
            }
            Ok(())
        })
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
        let mut next = self.source();
        let result = edit(&mut next)?;
        self.publish_change(next, resolver)?;
        Ok(result)
    }
    fn publish_change(
        &mut self,
        mut next: ContentSource,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        let before = self.current.clone();
        next.base.version = next
            .base
            .entities
            .iter()
            .map(EntityData::required_scene_version)
            .fold(next.base.version, u32::max);
        self.expand_source(&next, resolver)?;
        self.apply(next)?;
        if before.source != self.current.source {
            self.record(before);
        }
        Ok(())
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
    /// Insert one instance, optionally below an authored container, as one history state.
    pub fn insert_prefab(
        &mut self,
        container: Option<EntityData>,
        prefab: AssetId,
        resolver: &mut impl PrefabResolver,
    ) -> Result<InstanceId> {
        self.change(resolver, |next| {
            let parent = container.map(|container| {
                let id = container.id;
                next.base.entities.push(container);
                id
            });
            let id = InstanceId::new();
            next.instances.push(PrefabInstance {
                id,
                prefab,
                parent,
                overrides: vec![],
            });
            Ok(id)
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
