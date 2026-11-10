use crate::{
    FieldUpdate, SceneSnapshot,
    schema::{self, FieldKey, FieldValue},
};
use anyhow::{Context, Result, ensure};
use comet_core::{AssetId, EntityId, InstanceId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldOverride {
    pub entity: EntityId,
    pub field: FieldKey,
    pub value: FieldValue,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrefabInstance {
    pub id: InstanceId,
    pub prefab: AssetId,
    /// Attachment in this source's base scene; None attaches at the source root.
    pub parent: Option<EntityId>,
    pub overrides: Vec<FieldOverride>,
}
impl PrefabInstance {
    fn apply_overrides(&self, source: &mut ContentSource) -> Result<()> {
        if self.overrides.is_empty() {
            return Ok(());
        }
        source.base.version = self
            .overrides
            .iter()
            .fold(source.base.version, |version, value| {
                version
                    .max(schema::required_scene_version(value.field))
                    .max(schema::required_value_scene_version(&value.value))
            });
        let updates: Vec<_> = self
            .overrides
            .iter()
            .map(|value| FieldUpdate {
                entity: value.entity,
                field: value.field,
                value: value.value.clone(),
            })
            .collect();
        source
            .base
            .set_fields(&updates)
            .with_context(|| format!("overrides in instance {}", self.id))
    }
}
/// Authoring form shared by scene and prefab assets. Products retain instance declarations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentSource {
    pub version: u32,
    pub base: SceneSnapshot,
    pub instances: Vec<PrefabInstance>,
}
impl Default for ContentSource {
    fn default() -> Self {
        Self {
            version: 1,
            base: SceneSnapshot::default(),
            instances: vec![],
        }
    }
}
pub trait PrefabResolver {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource>;
}

/// An actual root's source identity and its relative instance path, from outer to inner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefabRoot {
    entity: EntityId,
    path: Vec<InstanceId>,
    requires_scene_root: bool,
}
impl PrefabRoot {
    /// Compose instance scopes before mapping the entity; UUID derivation is not associative.
    pub fn in_instance(&self, outer: InstanceId) -> EntityId {
        let scope = self
            .path
            .iter()
            .fold(outer, |scope, child| child.in_instance(scope));
        self.entity.in_instance(scope)
    }
    /// Physics bodies and navigation regions cannot attach below an authored container.
    pub fn requires_scene_root(&self) -> bool {
        self.requires_scene_root
    }
}

struct Expansion<'a> {
    scene: SceneSnapshot,
    roots: Option<&'a mut Vec<PrefabRoot>>,
    path: Vec<InstanceId>,
}

impl ContentSource {
    /// Detach one declaration, preserving expanded UUIDs and the nested instances' source scopes.
    /// `root` is the owning Prefab asset, when present, as in `expand_prefab`.
    pub fn detach_instance(
        &mut self,
        id: InstanceId,
        root: Option<AssetId>,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        let instance = self
            .instances
            .iter()
            .find(|instance| instance.id == id)
            .context("prefab instance is missing")?
            .clone();
        // Reject invalid references and self-use before lifting declarations out of their scope.
        self.expand_content(root, resolver, None)?;
        let mut content = resolver.prefab(instance.prefab)?;
        content.validate()?;
        instance.apply_overrides(&mut content)?;
        let entities: BTreeMap<_, _> = content
            .base
            .entities
            .iter()
            .map(|entity| (entity.id, entity.id.in_instance(id)))
            .collect();
        let instances: BTreeMap<_, _> = content
            .instances
            .iter()
            .map(|instance| (instance.id, instance.id.in_instance(id)))
            .collect();
        for entity in &mut content.base.entities {
            entity.id = entities[&entity.id];
            entity.parent = entity
                .parent
                .map(|parent| entities[&parent])
                .or(instance.parent);
            schema::remap(entity, &entities)?;
            schema::remap_instance_references(entity, &instances)?;
        }
        for nested in &mut content.instances {
            nested.id = instances[&nested.id];
            nested.parent = nested
                .parent
                .map(|parent| entities[&parent])
                .or(instance.parent);
            // Overrides still address the nested source; neither keys nor values are in our scope.
        }
        let mut candidate = self.clone();
        for entity in &mut candidate.base.entities {
            schema::map_entity_references(entity, |reference| {
                if let crate::EntityRef::InInstance { instance, entity } = reference
                    && *instance == id
                {
                    *reference = crate::EntityRef::local(entity.in_instance(id));
                }
                Ok(())
            })?;
        }
        candidate.base.version = candidate.base.version.max(content.base.version);
        candidate.base.entities.extend(content.base.entities);
        candidate.instances.retain(|instance| instance.id != id);
        candidate.instances.extend(content.instances);
        candidate.expand_content(root, resolver, None)?;
        candidate.canonicalize();
        *self = candidate;
        Ok(())
    }

    pub fn canonicalize(&mut self) {
        self.base.canonicalize();
        self.instances.sort_by_key(|i| i.id);
        for i in &mut self.instances {
            i.overrides
                .sort_by_key(|o| (o.entity, o.field.component, o.field.field));
        }
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let mut source: Self = serde_json::from_slice(bytes).context("parse content source")?;
        source.validate()?;
        source.canonicalize();
        Ok(source)
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut source = self.clone();
        source.canonicalize();
        Ok(serde_json::to_vec_pretty(&source)?)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported content version {}",
            self.version
        );
        self.base.validate()?;
        let mut ids = BTreeSet::new();
        for instance in &self.instances {
            ensure!(
                ids.insert(instance.id),
                "duplicate prefab instance {}",
                instance.id
            );
            if let Some(parent) = instance.parent {
                ensure!(
                    self.base.entities.iter().any(|e| e.id == parent),
                    "prefab {} missing attachment {parent}",
                    instance.id
                );
            }
            let mut overrides = BTreeSet::new();
            for o in &instance.overrides {
                schema::descriptor(o.field)?;
                let required_version = schema::required_scene_version(o.field)
                    .max(schema::required_value_scene_version(&o.value));
                ensure!(
                    required_version <= 2 || self.base.version >= required_version,
                    "prefab override requires scene version {}",
                    required_version
                );
                ensure!(
                    overrides.insert((o.entity, o.field.component, o.field.field)),
                    "duplicate prefab override"
                );
            }
        }
        for entity in &self.base.entities {
            for reference in schema::entity_references(entity) {
                if let crate::EntityRef::InInstance { instance, .. } = reference {
                    ensure!(
                        ids.contains(&instance),
                        "entity {} references missing prefab instance {instance}",
                        entity.id
                    );
                }
            }
        }
        Ok(())
    }
    pub fn dependencies(&self) -> BTreeSet<AssetId> {
        let mut deps = self.base.asset_dependencies();
        for i in &self.instances {
            deps.insert(i.prefab);
            for o in &i.overrides {
                match &o.value {
                    FieldValue::Asset(id) => {
                        deps.insert(*id);
                    }
                    FieldValue::OptionalAsset(Some(id)) => {
                        deps.insert(*id);
                    }
                    _ => {
                        for reference in schema::value_entity_references(&o.value) {
                            if let crate::EntityRef::External { scene, .. } = reference {
                                deps.insert(*scene);
                            }
                        }
                    }
                }
            }
        }
        deps
    }
    /// Expand declarations from snapshots; dependency discovery does not create an ECS World.
    pub fn expand(&self, resolver: &mut impl PrefabResolver) -> Result<SceneSnapshot> {
        self.expand_content(None, resolver, None)
    }
    /// Describe all expanded roots without binding them to a particular outer instance.
    pub fn prefab_roots(&self, resolver: &mut impl PrefabResolver) -> Result<Vec<PrefabRoot>> {
        let mut roots = vec![];
        self.expand_content(None, resolver, Some(&mut roots))?;
        roots.sort_by(|a, b| (&a.path, a.entity).cmp(&(&b.path, b.entity)));
        Ok(roots)
    }
    /// Seed the source identity before resolving children, so an older product cannot hide self-use.
    pub fn expand_prefab(
        &self,
        root: AssetId,
        resolver: &mut impl PrefabResolver,
    ) -> Result<SceneSnapshot> {
        self.expand_content(Some(root), resolver, None)
    }
    fn expand_content(
        &self,
        root: Option<AssetId>,
        resolver: &mut impl PrefabResolver,
        roots: Option<&mut Vec<PrefabRoot>>,
    ) -> Result<SceneSnapshot> {
        let mut output = Expansion {
            scene: SceneSnapshot {
                version: self.base.version,
                entities: vec![],
            },
            roots,
            path: vec![],
        };
        let mut stack = root.into_iter().collect();
        expand(self, None, None, resolver, &mut stack, &mut output)?;
        output.scene.validate_expanded()?;
        output.scene.canonicalize();
        Ok(output.scene)
    }
}
fn expand(
    source: &ContentSource,
    scope: Option<InstanceId>,
    mount: Option<EntityId>,
    resolver: &mut impl PrefabResolver,
    stack: &mut Vec<AssetId>,
    output: &mut Expansion<'_>,
) -> Result<()> {
    source.validate()?;
    output.scene.version = output.scene.version.max(source.base.version);
    ensure!(stack.len() < 64, "prefab nesting exceeds 64 levels");
    let map: BTreeMap<_, _> = source
        .base
        .entities
        .iter()
        .map(|e| (e.id, scope.map_or(e.id, |s| e.id.in_instance(s))))
        .collect();
    let instance_scopes: BTreeMap<_, _> = source
        .instances
        .iter()
        .map(|instance| {
            (
                instance.id,
                scope.map_or(instance.id, |parent| instance.id.in_instance(parent)),
            )
        })
        .collect();
    let mut targets: BTreeMap<InstanceId, BTreeSet<EntityId>> = BTreeMap::new();
    for source_entity in &source.base.entities {
        if source_entity.parent.is_none()
            && mount.is_none()
            && let Some(roots) = output.roots.as_mut()
        {
            roots.push(PrefabRoot {
                entity: source_entity.id,
                path: output.path.clone(),
                requires_scene_root: source_entity.physics.is_some()
                    || source_entity.navigation_region.is_some(),
            });
        }
        let mut e = source_entity.clone();
        e.id = map[&e.id];
        e.parent = e.parent.map(|p| map[&p]).or(mount);
        schema::map_entity_references(&mut e, |reference| {
            match reference {
                crate::EntityRef::Local { entity } => {
                    *entity = map[entity];
                }
                crate::EntityRef::InInstance { instance, entity } => {
                    targets.entry(*instance).or_default().insert(*entity);
                    let scope = instance_scopes
                        .get(instance)
                        .context("prefab instance reference is missing its declaration")?;
                    *reference = crate::EntityRef::local(entity.in_instance(*scope));
                }
                crate::EntityRef::External { .. } => {}
            }
            Ok(())
        })?;
        output.scene.entities.push(e);
    }
    for instance in &source.instances {
        ensure!(
            !stack.contains(&instance.prefab),
            "prefab dependency cycle: {stack:?} -> {}",
            instance.prefab
        );
        let mut child = resolver.prefab(instance.prefab).with_context(|| {
            format!("prefab instance {} asset {}", instance.id, instance.prefab)
        })?;
        child.validate()?;
        // Prove direct source membership, not merely a coincidental expanded UUID somewhere below it.
        if let Some(targets) = targets.get(&instance.id) {
            for target in targets {
                ensure!(
                    child
                        .base
                        .entities
                        .iter()
                        .any(|entity| entity.id == *target),
                    "prefab instance {} references missing direct source entity {target}",
                    instance.id
                );
            }
        }
        instance.apply_overrides(&mut child)?;
        let id = scope.map_or(instance.id, |s| instance.id.in_instance(s));
        let parent = instance.parent.map(|p| map[&p]).or(mount);
        stack.push(instance.prefab);
        if output.roots.is_some() {
            output.path.push(instance.id);
        }
        expand(&child, Some(id), parent, resolver, stack, output)?;
        if output.roots.is_some() {
            output.path.pop();
        }
        stack.pop();
    }
    Ok(())
}
