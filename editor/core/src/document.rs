use anyhow::{Context, Result, ensure};
use comet_core::{EntityId, InstanceId, atomic_write};
use comet_scene::{
    ContentSource, FieldOverride, PrefabResolver, Scene, SceneSnapshot,
    schema::{FieldKey, FieldValue},
};
use std::{collections::BTreeMap, path::Path};

pub struct Document {
    scene: Scene,
    instances: Vec<comet_scene::PrefabInstance>,
    saved: ContentSource,
    undo: Vec<ContentSource>,
    redo: Vec<ContentSource>,
    preview: Option<ContentSource>,
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
        Ok(Self {
            scene,
            instances: source.instances.clone(),
            saved: source,
            undo: vec![],
            redo: vec![],
            preview: None,
        })
    }
    pub fn load(path: &Path) -> Result<Self> {
        Self::open_source(ContentSource::from_json(
            &std::fs::read(path).with_context(|| format!("read {}", path.display()))?,
        )?)
    }
    pub fn snapshot(&mut self) -> SceneSnapshot {
        self.scene.snapshot()
    }
    pub fn world_id(&self) -> comet_core::WorldId {
        self.scene.world_id()
    }
    pub fn source(&mut self) -> ContentSource {
        ContentSource {
            version: 1,
            base: self.snapshot(),
            instances: self.instances.clone(),
        }
    }
    fn restore(&mut self, source: ContentSource) -> Result<()> {
        source.validate()?;
        self.scene.replace(source.base)?;
        self.instances = source.instances;
        Ok(())
    }
    pub fn is_dirty(&mut self) -> bool {
        self.source() != self.saved
    }
    pub fn history_len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_editing(&self) -> bool {
        self.preview.is_some()
    }
    pub fn begin(&mut self) -> Result<()> {
        ensure!(self.preview.is_none(), "transaction already active");
        self.preview = Some(self.source());
        Ok(())
    }
    pub fn preview_field(
        &mut self,
        selection: &[EntityId],
        field: FieldKey,
        value: FieldValue,
    ) -> Result<()> {
        ensure!(self.preview.is_some(), "no edit transaction");
        let mut next = self.snapshot();
        next.set_field(selection, field, value)?;
        self.scene.replace(next)
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
        self.restore(next)
    }
    pub fn commit(&mut self) -> Result<()> {
        let before = self.preview.take().context("no edit transaction")?;
        if before != self.source() {
            self.undo.push(before);
            self.redo.clear();
        }
        Ok(())
    }
    pub fn cancel(&mut self) -> Result<()> {
        let before = self.preview.take().context("no edit transaction")?;
        self.restore(before)
    }
    pub fn undo(&mut self) -> Result<()> {
        ensure!(self.preview.is_none(), "transaction active");
        let before = self.undo.last().context("nothing to undo")?.clone();
        let now = self.source();
        self.restore(before)?;
        self.undo.pop();
        self.redo.push(now);
        Ok(())
    }
    pub fn redo(&mut self) -> Result<()> {
        ensure!(self.preview.is_none(), "transaction active");
        let after = self.redo.last().context("nothing to redo")?.clone();
        let now = self.source();
        self.restore(after)?;
        self.redo.pop();
        self.undo.push(now);
        Ok(())
    }
    pub fn clone_subtrees(&mut self, roots: &[EntityId]) -> Result<BTreeMap<EntityId, EntityId>> {
        ensure!(self.preview.is_none(), "transaction active");
        let before = self.source();
        let mut next = before.clone();
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
        next.canonicalize();
        if !map.is_empty() {
            self.restore(next)?;
            self.undo.push(before);
            self.redo.clear();
        }
        Ok(map)
    }
    /// The savepoint moves only after the authoring file is published successfully.
    pub fn save(&mut self, path: &Path) -> Result<()> {
        ensure!(self.preview.is_none(), "cannot save an active transaction");
        let now = self.source();
        atomic_write(path, &now.to_json()?)?;
        self.saved = now;
        Ok(())
    }
}
