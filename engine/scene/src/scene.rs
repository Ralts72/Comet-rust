use crate::{
    EntityData, EntityRef, Links, Model, Transform,
    components::Identity,
    schema::{self, FieldKey, FieldValue},
};
use anyhow::{Context, Result, ensure};
use bevy_ecs::world::World;
use comet_core::{AssetId, EntityId, WorldId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const SCENE_VERSION: u32 = 1;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSnapshot {
    pub version: u32,
    pub entities: Vec<EntityData>,
}
impl Default for SceneSnapshot {
    fn default() -> Self {
        Self {
            version: SCENE_VERSION,
            entities: vec![],
        }
    }
}
impl SceneSnapshot {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == SCENE_VERSION,
            "unsupported scene version {}",
            self.version
        );
        let mut parents = BTreeMap::new();
        for e in &self.entities {
            ensure!(
                parents.insert(e.id, e.parent).is_none(),
                "duplicate entity {}",
                e.id
            );
            schema::validate(e)?;
        }
        for e in &self.entities {
            let mut chain = BTreeSet::from([e.id]);
            let mut next = e.parent;
            while let Some(p) = next {
                ensure!(chain.insert(p), "hierarchy cycle at {p}");
                next = *parents
                    .get(&p)
                    .with_context(|| format!("entity {} missing parent {p}", e.id))?;
            }
            for r in schema::entity_references(e) {
                if let EntityRef::Local { entity: target } = r {
                    ensure!(
                        parents.contains_key(&target),
                        "entity {} references missing local entity {target}",
                        e.id
                    );
                }
            }
        }
        Ok(())
    }
    pub fn canonicalize(&mut self) {
        self.entities.sort_by_key(|e| e.id);
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let mut value: Self = serde_json::from_slice(bytes).context("parse scene")?;
        value.validate()?;
        value.canonicalize();
        Ok(value)
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut value = self.clone();
        value.canonicalize();
        Ok(serde_json::to_vec_pretty(&value)?)
    }
    pub fn asset_dependencies(&self) -> BTreeSet<AssetId> {
        self.entities
            .iter()
            .flat_map(schema::asset_references)
            .collect()
    }
    pub fn reparent(&mut self, id: EntityId, parent: Option<EntityId>) -> Result<()> {
        let mut candidate = self.clone();
        candidate
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .context("stale hierarchy selection")?
            .parent = parent;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn set_field(
        &mut self,
        selection: &[EntityId],
        key: FieldKey,
        value: FieldValue,
    ) -> Result<()> {
        let field = schema::descriptor(key)?;
        let mut candidate = self.clone();
        let mut unique = BTreeSet::new();
        for id in selection {
            ensure!(unique.insert(*id), "duplicate selection {id}");
            let e = candidate
                .entities
                .iter_mut()
                .find(|e| e.id == *id)
                .with_context(|| format!("stale selection {id}"))?;
            field
                .write(e, value.clone())
                .with_context(|| format!("{id}: {}", field.path))?;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Copy roots and descendants in the same scene, preserving references outside the selection.
    pub fn clone_subtrees(&mut self, roots: &[EntityId]) -> Result<BTreeMap<EntityId, EntityId>> {
        self.validate()?;
        let mut selected: BTreeSet<_> = roots.iter().copied().collect();
        ensure!(selected.len() == roots.len(), "duplicate clone root");
        for id in roots {
            ensure!(
                self.entities.iter().any(|e| e.id == *id),
                "stale clone source {id}"
            );
        }
        loop {
            let before = selected.len();
            for e in &self.entities {
                if e.parent.is_some_and(|p| selected.contains(&p)) {
                    selected.insert(e.id);
                }
            }
            if selected.len() == before {
                break;
            }
        }
        let map: BTreeMap<_, _> = selected
            .into_iter()
            .map(|id| (id, EntityId::new()))
            .collect();
        let mut candidate = self.clone();
        for source in &self.entities {
            if let Some(id) = map.get(&source.id) {
                let mut e = source.clone();
                e.id = *id;
                e.parent = e.parent.map(|p| map.get(&p).copied().unwrap_or(p));
                schema::remap(&mut e, &map)?;
                candidate.entities.push(e);
            }
        }
        candidate.validate()?;
        candidate.canonicalize();
        *self = candidate;
        Ok(map)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityHandle {
    world: WorldId,
    generation: u64,
    id: EntityId,
}
/// Content container, independent of history and active simulation services.
pub struct Scene {
    world: World,
    id: WorldId,
    generation: u64,
}
impl Scene {
    pub fn open(snapshot: SceneSnapshot) -> Result<Self> {
        let mut scene = Self {
            world: World::new(),
            id: WorldId::new(),
            generation: 0,
        };
        scene.replace(snapshot)?;
        Ok(scene)
    }
    pub fn world_id(&self) -> WorldId {
        self.id
    }
    pub fn replace(&mut self, snapshot: SceneSnapshot) -> Result<()> {
        snapshot.validate()?;
        let generation = self
            .generation
            .checked_add(1)
            .context("scene generation exhausted")?;
        let mut world = World::new();
        for e in snapshot.entities {
            let mut entity = world.spawn((
                Identity {
                    id: e.id,
                    name: e.name,
                    parent: e.parent,
                },
                e.transform,
                e.links,
            ));
            if let Some(model) = e.model {
                entity.insert(model);
            }
        }
        self.world = world;
        self.generation = generation;
        Ok(())
    }
    pub fn snapshot(&mut self) -> SceneSnapshot {
        let mut entities: Vec<_> = self
            .world
            .query::<(&Identity, &Transform, Option<&Model>, &Links)>()
            .iter(&self.world)
            .map(|(i, t, m, l)| EntityData {
                id: i.id,
                name: i.name.clone(),
                parent: i.parent,
                transform: t.clone(),
                model: m.cloned(),
                links: l.clone(),
            })
            .collect();
        entities.sort_by_key(|e| e.id);
        SceneSnapshot {
            version: SCENE_VERSION,
            entities,
        }
    }
    pub fn handle(&mut self, id: EntityId) -> Result<EntityHandle> {
        ensure!(
            self.world
                .query::<&Identity>()
                .iter(&self.world)
                .any(|e| e.id == id),
            "entity does not exist {id}"
        );
        Ok(EntityHandle {
            world: self.id,
            generation: self.generation,
            id,
        })
    }
    pub fn resolve(&mut self, h: EntityHandle) -> Result<EntityData> {
        ensure!(
            h.world == self.id && h.generation == self.generation,
            "stale or foreign entity handle"
        );
        self.snapshot()
            .entities
            .into_iter()
            .find(|e| e.id == h.id)
            .context("entity removed")
    }
}
