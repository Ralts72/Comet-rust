use crate::{
    Character, EntityData, EntityRef, HingeJoint, Links, Model, NavigationAgent, NavigationRegion,
    PhysicsBody, PointJoint, Script, SliderJoint, Transform,
    components::Identity,
    schema::{self, FieldKey, FieldValue},
};
use anyhow::{Context, Result, ensure};
use bevy_ecs::world::World;
use comet_core::{AssetId, EntityId, WorldId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub const SCENE_VERSION: u32 = 10;
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
        crate::world_transforms(self).map(|_| ())
    }
    /// Consumers of expanded worlds cannot interpret authoring instance declarations.
    pub fn validate_expanded(&self) -> Result<()> {
        self.validate()?;
        for entity in &self.entities {
            ensure!(
                schema::entity_references(entity)
                    .iter()
                    .all(|reference| !matches!(reference, EntityRef::InInstance { .. })),
                "entity {} contains an unresolved prefab instance reference",
                entity.id
            );
        }
        Ok(())
    }

    pub(crate) fn validate_structure(&self) -> Result<()> {
        ensure!(
            (1..=SCENE_VERSION).contains(&self.version),
            "unsupported scene version {}",
            self.version
        );
        ensure!(
            self.entities
                .iter()
                .filter(|e| e.character.as_ref().is_some_and(|c| c.player_controlled))
                .count()
                <= 1,
            "scene has multiple player-controlled characters"
        );
        ensure!(
            self.entities
                .iter()
                .filter(|e| e.navigation_region.is_some())
                .count()
                <= 1,
            "scene has multiple navigation regions"
        );
        let mut parents = BTreeMap::new();
        for e in &self.entities {
            ensure!(
                parents.insert(e.id, e.parent).is_none(),
                "duplicate entity {}",
                e.id
            );
            let required = e.required_scene_version();
            ensure!(
                self.version >= required,
                "entity {} requires scene version {required}",
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
        crate::joints::validate_connections(self)
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
        schema::descriptor(key)?;
        self.set_fields(
            &selection
                .iter()
                .map(|entity| FieldUpdate {
                    entity: *entity,
                    field: key,
                    value: value.clone(),
                })
                .collect::<Vec<_>>(),
        )
    }
    pub fn set_fields(&mut self, updates: &[FieldUpdate]) -> Result<()> {
        let mut candidate = self.clone();
        candidate.write_fields(updates)?;
        *self = candidate;
        Ok(())
    }
    /// Only unpublished candidates call this; callers discard the candidate on any error.
    fn write_fields(&mut self, updates: &[FieldUpdate]) -> Result<()> {
        let mut unique = BTreeSet::new();
        for update in updates {
            let id = update.entity;
            ensure!(
                unique.insert((id, update.field.component, update.field.field)),
                "duplicate entity field update {id}"
            );
            let field = schema::descriptor(update.field)?;
            let e = self
                .entities
                .iter_mut()
                .find(|e| e.id == id)
                .with_context(|| format!("stale selection {id}"))?;
            field
                .write(e, update.value.clone())
                .with_context(|| format!("{id}: {}", field.path))?;
        }
        self.validate()
    }
    /// Copy roots and descendants in the same scene, preserving references outside the selection.
    pub fn clone_subtrees(&mut self, roots: &[EntityId]) -> Result<BTreeMap<EntityId, EntityId>> {
        Ok(self
            .clone_subtrees_many(roots, 1)?
            .pop()
            .expect("one copy requested"))
    }
    /// Collect the original subtrees once; each copy gets its own reference namespace.
    pub fn clone_subtrees_many(
        &mut self,
        roots: &[EntityId],
        copies: usize,
    ) -> Result<Vec<BTreeMap<EntityId, EntityId>>> {
        let selected = self.subtree_ids(roots)?;
        let mut candidate = self.clone();
        let mut maps = Vec::new();
        for _ in 0..copies {
            let map: BTreeMap<_, _> = selected.iter().map(|id| (*id, EntityId::new())).collect();
            for source in &self.entities {
                if let Some(id) = map.get(&source.id) {
                    let mut entity = source.clone();
                    entity.id = *id;
                    entity.parent = entity
                        .parent
                        .map(|parent| map.get(&parent).copied().unwrap_or(parent));
                    schema::remap(&mut entity, &map)?;
                    candidate.entities.push(entity);
                }
            }
            maps.push(map);
        }
        candidate.validate()?;
        candidate.canonicalize();
        *self = candidate;
        Ok(maps)
    }
    pub fn remove_subtrees(&mut self, roots: &[EntityId]) -> Result<BTreeSet<EntityId>> {
        let removed = self.subtree_ids(roots)?;
        let mut candidate = self.clone();
        candidate
            .entities
            .retain(|entity| !removed.contains(&entity.id));
        for entity in &mut candidate.entities {
            schema::clear_removed_references(entity, &removed)?;
        }
        candidate.validate()?;
        candidate.canonicalize();
        *self = candidate;
        Ok(removed)
    }
    /// Validate the source and collect the requested authored roots and all their descendants.
    pub fn subtree_ids(&self, roots: &[EntityId]) -> Result<BTreeSet<EntityId>> {
        self.validate()?;
        let mut selected: BTreeSet<_> = roots.iter().copied().collect();
        ensure!(selected.len() == roots.len(), "duplicate object root");
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
        Ok(selected)
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
    version: u32,
    revision: u64,
    cached: Option<Arc<SceneSnapshot>>,
}
impl Scene {
    pub fn open(snapshot: SceneSnapshot) -> Result<Self> {
        let mut scene = Self {
            world: World::new(),
            id: WorldId::new(),
            generation: 0,
            version: snapshot.version,
            revision: 0,
            cached: None,
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
        let revision = self
            .revision
            .checked_add(1)
            .context("scene revision exhausted")?;
        let version = snapshot.version;
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
            if let Some(body) = e.physics {
                entity.insert(body);
            }
            if let Some(character) = e.character {
                entity.insert(character);
            }
            if let Some(model) = e.model {
                entity.insert(model);
            }
            if let Some(script) = e.script {
                entity.insert(script);
            }
            if let Some(joint) = e.point_joint {
                entity.insert(joint);
            }
            if let Some(joint) = e.hinge_joint {
                entity.insert(joint);
            }
            if let Some(joint) = e.slider_joint {
                entity.insert(joint);
            }
            if let Some(region) = e.navigation_region {
                entity.insert(region);
            }
            if let Some(agent) = e.navigation_agent {
                entity.insert(agent);
            }
            if let Some(audio) = e.audio_source {
                entity.insert(audio);
            }
            if let Some(animator) = e.animator {
                entity.insert(animator);
            }
            if let Some(skin) = e.skin {
                entity.insert(skin);
            }
        }
        self.revision = revision;
        self.cached = None;
        self.version = version;
        self.world = world;
        self.generation = generation;
        Ok(())
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn snapshot(&mut self) -> SceneSnapshot {
        self.shared_snapshot().as_ref().clone()
    }
    /// Commit a complete field candidate without changing entity generations or invalidating handles.
    pub fn set_fields(&mut self, updates: &[FieldUpdate]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        let before = self.shared_snapshot();
        let mut candidate = before.as_ref().clone();
        candidate.write_fields(updates)?;
        if candidate == *before {
            return Ok(());
        }
        let revision = self
            .revision
            .checked_add(1)
            .context("scene revision exhausted")?;
        let ids: BTreeSet<_> = updates.iter().map(|update| update.entity).collect();
        let entities: BTreeMap<_, _> = self
            .world
            .query::<(bevy_ecs::entity::Entity, &Identity)>()
            .iter(&self.world)
            .filter(|(_, identity)| ids.contains(&identity.id))
            .map(|(entity, identity)| (identity.id, entity))
            .collect();
        for data in candidate
            .entities
            .iter()
            .filter(|entity| ids.contains(&entity.id))
        {
            let mut entity = self.world.entity_mut(entities[&data.id]);
            entity.insert((data.transform.clone(), data.links.clone()));
            if let Some(model) = &data.model {
                entity.insert(model.clone());
            }
            if let Some(body) = &data.physics {
                entity.insert(body.clone());
            }
            if let Some(character) = &data.character {
                entity.insert(character.clone());
            }
            if let Some(script) = &data.script {
                entity.insert(script.clone());
            }
            if let Some(joint) = &data.point_joint {
                entity.insert(joint.clone());
            }
            if let Some(joint) = &data.hinge_joint {
                entity.insert(joint.clone());
            }
            if let Some(joint) = &data.slider_joint {
                entity.insert(joint.clone());
            }
            if let Some(region) = &data.navigation_region {
                entity.insert(region.clone());
            }
            if let Some(agent) = &data.navigation_agent {
                entity.insert(agent.clone());
            }
            if let Some(audio) = &data.audio_source {
                entity.insert(audio.clone());
            }
            if let Some(animator) = &data.animator {
                entity.insert(animator.clone());
            }
            if let Some(skin) = &data.skin {
                entity.insert(skin.clone());
            }
        }
        self.revision = revision;
        self.cached = Some(Arc::new(candidate));
        Ok(())
    }
    pub fn shared_snapshot(&mut self) -> Arc<SceneSnapshot> {
        if let Some(snapshot) = &self.cached {
            return snapshot.clone();
        }
        let mut entities: Vec<_> = self
            .world
            .query::<(
                &Identity,
                &Transform,
                Option<&Model>,
                &Links,
                Option<&PhysicsBody>,
                Option<&Character>,
                Option<&Script>,
                Option<&PointJoint>,
                Option<&HingeJoint>,
                Option<&SliderJoint>,
                Option<&NavigationRegion>,
                Option<&NavigationAgent>,
                Option<&crate::AudioSource>,
                Option<&crate::Animator>,
                Option<&crate::SkinBinding>,
            )>()
            .iter(&self.world)
            .map(
                |(
                    i,
                    t,
                    m,
                    l,
                    p,
                    c,
                    s,
                    point,
                    hinge,
                    slider,
                    region,
                    agent,
                    audio,
                    animator,
                    skin,
                )| EntityData {
                    id: i.id,
                    name: i.name.clone(),
                    parent: i.parent,
                    transform: t.clone(),
                    model: m.cloned(),
                    links: l.clone(),
                    physics: p.cloned(),
                    character: c.cloned(),
                    script: s.cloned(),
                    point_joint: point.cloned(),
                    hinge_joint: hinge.cloned(),
                    slider_joint: slider.cloned(),
                    navigation_region: region.cloned(),
                    navigation_agent: agent.cloned(),
                    audio_source: audio.cloned(),
                    animator: animator.cloned(),
                    skin: skin.cloned(),
                },
            )
            .collect();
        entities.sort_by_key(|e| e.id);
        let snapshot = Arc::new(SceneSnapshot {
            version: self.version,
            entities,
        });
        self.cached = Some(snapshot.clone());
        snapshot
    }
    /// Validate the entire batch before changing any component; existing handles remain valid.
    pub fn update_poses(&mut self, updates: &[PoseUpdate]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        let mut pending = BTreeMap::new();
        for update in updates {
            ensure!(
                pending.insert(update.entity, update).is_none(),
                "duplicate pose update"
            );
            schema::validate_pose(update.translation, update.rotation)?;
        }
        let mut entities = BTreeMap::new();
        for (entity, identity, character) in self
            .world
            .query::<(bevy_ecs::entity::Entity, &Identity, Option<&Character>)>()
            .iter(&self.world)
        {
            if let Some(update) = pending.get(&identity.id) {
                ensure!(
                    identity.parent.is_none(),
                    "pose update requires a root entity"
                );
                if character.is_some() {
                    ensure!(
                        update.rotation[0].abs() < 1e-5 && update.rotation[2].abs() < 1e-5,
                        "character must remain upright"
                    );
                }
                entities.insert(identity.id, entity);
            }
        }
        ensure!(
            pending.keys().all(|id| entities.contains_key(id)),
            "stale pose update"
        );
        let changed = pending.iter().any(|(id, u)| {
            let transform = self
                .world
                .get::<Transform>(entities[id])
                .expect("scene transform");
            transform.translation != u.translation || transform.rotation != u.rotation
        });
        if !changed {
            return Ok(());
        }
        {
            let mut query = self.world.query::<(&Identity, &Transform)>();
            let nodes: Vec<_> = query
                .iter(&self.world)
                .map(|(identity, transform)| crate::transform::TransformNode {
                    id: identity.id,
                    parent: identity.parent,
                    transform,
                })
                .collect();
            crate::transform::compose_world_transforms(&nodes, &pending)?;
        }
        let revision = self
            .revision
            .checked_add(1)
            .context("scene revision exhausted")?;
        for (id, update) in pending {
            let mut transform = self
                .world
                .get_mut::<Transform>(entities[&id])
                .expect("scene transform");
            transform.translation = update.translation;
            transform.rotation = update.rotation;
        }
        self.revision = revision;
        self.cached = None;
        Ok(())
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
        self.shared_snapshot()
            .entities
            .iter()
            .find(|e| e.id == h.id)
            .cloned()
            .context("entity removed")
    }
}

/// World-space pose for a root entity. Scale remains authored content.
#[derive(Clone, Debug)]
pub struct PoseUpdate {
    pub entity: EntityId,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct FieldUpdate {
    pub entity: EntityId,
    pub field: FieldKey,
    pub value: FieldValue,
}
