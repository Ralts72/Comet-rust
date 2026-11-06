use super::Document;
use anyhow::{Context, Result, ensure};
use comet_core::{EntityId, InstanceId};
use comet_scene::{
    ContentSource, FieldOverride, PrefabResolver,
    schema::{self, FieldKey, FieldValue},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectTarget {
    Authored(EntityId),
    Prefab(InstanceId),
}

#[derive(Default, Debug)]
pub struct CloneMap {
    pub entities: BTreeMap<EntityId, EntityId>,
    pub instances: BTreeMap<InstanceId, InstanceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FieldTarget {
    Authored(EntityId),
    Prefab {
        instance: InstanceId,
        entity: EntityId,
    },
}

#[derive(Clone, Debug)]
pub struct FieldEdit {
    pub target: FieldTarget,
    pub field: FieldKey,
    pub value: FieldValue,
}

#[derive(Clone, Debug)]
pub struct OverrideReset {
    pub instance: InstanceId,
    pub entity: EntityId,
    pub fields: Vec<FieldKey>,
}

impl Document {
    /// Remove explicit declarations on a direct prefab source entity, restoring inheritance.
    /// An existing declaration can be removed even if its source component was removed.
    pub fn revert_overrides(
        &mut self,
        instance: InstanceId,
        entity: EntityId,
        fields: &[FieldKey],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.revert_overrides_many(
            &[OverrideReset {
                instance,
                entity,
                fields: fields.to_vec(),
            }],
            resolver,
        )
    }

    /// Restore all selected source declarations in one fully validated history change.
    pub fn revert_overrides_many(
        &mut self,
        resets: &[OverrideReset],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        ensure!(self.preview.is_none(), "transaction active");
        if resets.is_empty() {
            return Ok(());
        }
        let mut targets = BTreeMap::new();
        for reset in resets {
            let target = self
                .current
                .source
                .instances
                .iter()
                .find(|value| value.id == reset.instance)
                .context("stale prefab override selection")?;
            let prefab = resolver.prefab(target.prefab)?;
            let source_entity = prefab
                .base
                .entities
                .iter()
                .find(|value| value.id == reset.entity)
                .context("override reset requires a direct prefab source entity")?;
            let mut keys = BTreeSet::new();
            for &field in &reset.fields {
                ensure!(
                    keys.insert((field.component, field.field)),
                    "duplicate override reset field"
                );
                let descriptor = schema::descriptor(field)?;
                ensure!(
                    descriptor.read(source_entity).is_some()
                        || target
                            .overrides
                            .iter()
                            .any(|value| value.entity == reset.entity && value.field == field),
                    "prefab source entity has no field {}",
                    descriptor.path
                );
            }
            ensure!(
                targets
                    .insert((reset.instance, reset.entity), keys)
                    .is_none(),
                "duplicate override reset target"
            );
        }
        self.change(resolver, |next| {
            for instance in &mut next.instances {
                let id = instance.id;
                instance.overrides.retain(|value| {
                    !targets.get(&(id, value.entity)).is_some_and(|keys| {
                        keys.contains(&(value.field.component, value.field.field))
                    })
                });
            }
            Ok(())
        })
    }

    /// Validate one complete field candidate, including prefab expansion, before replacing a preview.
    pub fn preview_fields(
        &mut self,
        edits: &[FieldEdit],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        ensure!(self.preview.is_some(), "no edit transaction");
        let mut next = self.source();
        write_fields(&mut next, edits)?;
        self.expand_source(&next, resolver)?;
        self.apply(next)
    }

    /// Preview inspector values without changing the transaction's original field declarations
    /// when a value returns to its starting value. Other field previews remain intact.
    /// Explicit overrides belong to `preview_override`/`preview_fields`; this does not prune
    /// values equal to the current prefab source or alter previously saved pins.
    pub fn preview_field_values(
        &mut self,
        edits: &[FieldEdit],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        let before = self
            .preview
            .as_ref()
            .context("no edit transaction")?
            .clone();
        let mut next = self.source();
        write_fields(&mut next, edits)?;
        for edit in edits {
            let initial = initial_field_value(&before.source, edit, resolver)?;
            if initial.as_ref() != Some(&edit.value) {
                continue;
            }
            if let FieldTarget::Prefab { instance, entity } = edit.target {
                let original = before
                    .source
                    .instances
                    .iter()
                    .find(|value| value.id == instance)
                    .and_then(|value| {
                        value
                            .overrides
                            .iter()
                            .find(|value| value.entity == entity && value.field == edit.field)
                    });
                let target = next
                    .instances
                    .iter_mut()
                    .find(|value| value.id == instance)
                    .context("stale prefab field selection")?;
                if let Some(original) = original {
                    // write_fields already inserted this key in canonical order.
                    *target
                        .overrides
                        .iter_mut()
                        .find(|value| value.entity == entity && value.field == edit.field)
                        .context("missing preview field declaration")? = original.clone();
                } else {
                    target
                        .overrides
                        .retain(|value| value.entity != entity || value.field != edit.field);
                }
            }
        }
        let version = next.base.version;
        next.base.version = before.source.base.version;
        let unchanged = next == *before.source;
        if !unchanged {
            // Preserve upgrades while any other preview remains; do not infer a minimum protocol.
            next.base.version = version;
        }
        self.expand_source(&next, resolver)?;
        if unchanged {
            self.restore(before)
        } else {
            self.apply(next)
        }
    }

    /// Replace a gesture preview from its initial declaration, preserving omitted prefab overrides.
    pub fn preview_fields_from_baseline(
        &mut self,
        edits: &[FieldEdit],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        let mut next = self
            .preview
            .as_ref()
            .context("no edit transaction")?
            .source
            .as_ref()
            .clone();
        write_fields(&mut next, edits)?;
        self.expand_source(&next, resolver)?;
        self.apply(next)
    }

    /// Duplicate each selected subtree and instance once, including instances attached below a subtree.
    pub fn clone_selection(
        &mut self,
        selection: &[ObjectTarget],
        resolver: &mut impl PrefabResolver,
    ) -> Result<CloneMap> {
        self.change(resolver, |next| {
            let (roots, instances) = selection_targets(next, selection)?;
            let entities = next.base.clone_subtrees(&roots)?;
            let mut result = CloneMap {
                entities,
                instances: BTreeMap::new(),
            };
            let mut copies = Vec::new();
            for instance in &next.instances {
                let parent = instance
                    .parent
                    .and_then(|id| result.entities.get(&id).copied());
                if parent.is_some() || instances.contains(&instance.id) {
                    let mut copy = instance.clone();
                    copy.id = InstanceId::new();
                    copy.parent = parent.or(instance.parent);
                    result.instances.insert(instance.id, copy.id);
                    copies.push(copy);
                }
            }
            next.instances.extend(copies);
            Ok(result)
        })
    }

    pub fn remove_selection(
        &mut self,
        selection: &[ObjectTarget],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.change(resolver, |next| {
            let (roots, instances) = selection_targets(next, selection)?;
            let removed = next.base.remove_subtrees(&roots)?;
            next.instances.retain(|instance| {
                !instances.contains(&instance.id)
                    && !instance
                        .parent
                        .is_some_and(|parent| removed.contains(&parent))
            });
            Ok(())
        })
    }

    pub fn clone_subtrees(
        &mut self,
        roots: &[EntityId],
        resolver: &mut impl PrefabResolver,
    ) -> Result<BTreeMap<EntityId, EntityId>> {
        Ok(self
            .clone_selection(
                &roots
                    .iter()
                    .copied()
                    .map(ObjectTarget::Authored)
                    .collect::<Vec<_>>(),
                resolver,
            )?
            .entities)
    }

    pub fn remove_subtrees(
        &mut self,
        roots: &[EntityId],
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.remove_selection(
            &roots
                .iter()
                .copied()
                .map(ObjectTarget::Authored)
                .collect::<Vec<_>>(),
            resolver,
        )
    }

    pub fn duplicate_instance(
        &mut self,
        id: InstanceId,
        resolver: &mut impl PrefabResolver,
    ) -> Result<InstanceId> {
        self.clone_selection(&[ObjectTarget::Prefab(id)], resolver)?
            .instances
            .get(&id)
            .copied()
            .context("missing duplicated instance")
    }

    pub fn remove_instance(
        &mut self,
        id: InstanceId,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.remove_selection(&[ObjectTarget::Prefab(id)], resolver)
    }
}

pub(super) fn write_fields(source: &mut ContentSource, edits: &[FieldEdit]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for edit in edits {
        ensure!(
            unique.insert((edit.target, edit.field.component, edit.field.field)),
            "duplicate field edit"
        );
        let field = schema::descriptor(edit.field)?;
        source.base.version = source
            .base
            .version
            .max(schema::required_scene_version(edit.field))
            .max(schema::required_value_scene_version(&edit.value));
        match edit.target {
            FieldTarget::Authored(id) => {
                let entity = source
                    .base
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == id)
                    .context("stale authored field selection")?;
                field.write(entity, edit.value.clone())?;
            }
            FieldTarget::Prefab { instance, entity } => {
                let target = source
                    .instances
                    .iter_mut()
                    .find(|target| target.id == instance)
                    .context("stale prefab field selection")?;
                target
                    .overrides
                    .retain(|value| value.entity != entity || value.field != edit.field);
                target.overrides.push(FieldOverride {
                    entity,
                    field: edit.field,
                    value: edit.value.clone(),
                });
            }
        }
    }
    source.canonicalize();
    Ok(())
}

fn initial_field_value(
    source: &ContentSource,
    edit: &FieldEdit,
    resolver: &mut impl PrefabResolver,
) -> Result<Option<FieldValue>> {
    let field = schema::descriptor(edit.field)?;
    match edit.target {
        FieldTarget::Authored(id) => Ok(source
            .base
            .entities
            .iter()
            .find(|entity| entity.id == id)
            .and_then(|entity| field.read(entity))),
        FieldTarget::Prefab { instance, entity } => {
            let Some(instance) = source.instances.iter().find(|value| value.id == instance) else {
                return Ok(None);
            };
            if let Some(value) = instance
                .overrides
                .iter()
                .find(|value| value.entity == entity && value.field == edit.field)
            {
                return Ok(Some(value.value.clone()));
            }
            // Read source IDs before instance remapping, including entity references in parameters.
            let prefab = resolver.prefab(instance.prefab)?;
            Ok(prefab
                .base
                .entities
                .iter()
                .find(|value| value.id == entity)
                .and_then(|entity| field.read(entity)))
        }
    }
}

fn selection_targets(
    source: &ContentSource,
    selection: &[ObjectTarget],
) -> Result<(Vec<EntityId>, BTreeSet<InstanceId>)> {
    let mut roots = BTreeSet::new();
    let mut instances = BTreeSet::new();
    for target in selection {
        match *target {
            ObjectTarget::Authored(id) => {
                ensure!(
                    source.base.entities.iter().any(|entity| entity.id == id),
                    "stale authored object selection"
                );
                roots.insert(id);
            }
            ObjectTarget::Prefab(id) => {
                ensure!(
                    source.instances.iter().any(|instance| instance.id == id),
                    "stale prefab instance selection"
                );
                instances.insert(id);
            }
        }
    }
    Ok((roots.into_iter().collect(), instances))
}
