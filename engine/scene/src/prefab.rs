use crate::{
    SceneSnapshot,
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
impl ContentSource {
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
                ensure!(
                    overrides.insert((o.entity, o.field.component, o.field.field)),
                    "duplicate prefab override"
                );
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
                    FieldValue::Entity(Some(crate::EntityRef::External { scene, .. })) => {
                        deps.insert(*scene);
                    }
                    FieldValue::Entities(refs) => {
                        for r in refs {
                            if let crate::EntityRef::External { scene, .. } = r {
                                deps.insert(*scene);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        deps
    }
    /// Expand declarations from snapshots; dependency discovery does not create an ECS World.
    pub fn expand(&self, resolver: &mut impl PrefabResolver) -> Result<SceneSnapshot> {
        let mut result = SceneSnapshot::default();
        expand(self, None, None, resolver, &mut vec![], &mut result)?;
        result.validate()?;
        result.canonicalize();
        Ok(result)
    }
}
fn expand(
    source: &ContentSource,
    scope: Option<InstanceId>,
    mount: Option<EntityId>,
    resolver: &mut impl PrefabResolver,
    stack: &mut Vec<AssetId>,
    output: &mut SceneSnapshot,
) -> Result<()> {
    source.validate()?;
    ensure!(stack.len() < 64, "prefab nesting exceeds 64 levels");
    let map: BTreeMap<_, _> = source
        .base
        .entities
        .iter()
        .map(|e| (e.id, scope.map_or(e.id, |s| e.id.in_instance(s))))
        .collect();
    for source_entity in &source.base.entities {
        let mut e = source_entity.clone();
        e.id = map[&e.id];
        e.parent = e.parent.map(|p| map[&p]).or(mount);
        schema::remap(&mut e, &map)?;
        output.entities.push(e);
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
        for o in &instance.overrides {
            child
                .base
                .set_field(&[o.entity], o.field, o.value.clone())
                .with_context(|| format!("override in instance {}", instance.id))?;
        }
        let id = scope.map_or(instance.id, |s| instance.id.in_instance(s));
        let parent = instance.parent.map(|p| map[&p]).or(mount);
        stack.push(instance.prefab);
        expand(&child, Some(id), parent, resolver, stack, output)?;
        stack.pop();
    }
    Ok(())
}
