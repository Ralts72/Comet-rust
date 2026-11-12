use super::{CloneMap, Document, ObjectTarget, selection::clone_objects};
use anyhow::{Context, Result, ensure};
use comet_core::{
    EntityId,
    math::{DVec3, Mat4, Vec3},
};
use comet_scene::{ContentSource, PrefabResolver, checked_transform_inverse, world_transforms};
use std::collections::BTreeMap;

pub const MAX_PLACEMENT_COPIES: u32 = 1000;

impl Document {
    /// Copy authored subtrees and linked instances by a world-space step in one history state.
    pub fn place_copies(
        &mut self,
        selection: &[ObjectTarget],
        copies: u32,
        world_step: [f32; 3],
        resolver: &mut impl PrefabResolver,
    ) -> Result<Vec<CloneMap>> {
        ensure!(self.preview.is_none(), "transaction active");
        ensure!(
            (1..=MAX_PLACEMENT_COPIES).contains(&copies),
            "copies must be in 1..={MAX_PLACEMENT_COPIES}"
        );
        let step = Vec3::from(world_step);
        ensure!(step.is_finite(), "placement step must be finite");
        let before = self.shared_source();
        let mut next = self.source();
        let maps = clone_objects(&mut next, selection, copies as usize)?;
        if step == Vec3::ZERO {
            // Exact copies need neither an invertible attachment nor representable displacement.
            self.publish_change(next, resolver, |_| Ok(()))?;
            return Ok(maps);
        }
        let original = world_transforms(&self.expand(resolver)?)?;
        offset_copies(&mut next, &maps, step, &original)?;
        let instances: BTreeMap<_, _> = before
            .instances
            .iter()
            .filter(|instance| maps[0].instances.contains_key(&instance.id))
            .map(|instance| (instance.id, instance.prefab))
            .collect();
        let mut roots = BTreeMap::new();
        for &asset in instances.values() {
            if let std::collections::btree_map::Entry::Vacant(entry) = roots.entry(asset) {
                entry.insert(resolver.prefab(asset)?.prefab_roots(resolver)?);
            }
        }
        self.publish_change(next, resolver, |snapshot| {
            let placed = world_transforms(snapshot)?;
            for (index, map) in maps.iter().enumerate() {
                let expected = step.as_dvec3() * (index + 1) as f64;
                for (&old, &new) in &map.entities {
                    check_displacement(&original, &placed, old, new, expected)?;
                }
                for (&old, &new) in &map.instances {
                    for root in &roots[&instances[&old]] {
                        check_displacement(
                            &original,
                            &placed,
                            root.in_instance(old),
                            root.in_instance(new),
                            expected,
                        )?;
                    }
                }
            }
            Ok(())
        })?;
        Ok(maps)
    }
}

fn offset_copies(
    source: &mut ContentSource,
    maps: &[CloneMap],
    step: Vec3,
    world: &BTreeMap<EntityId, Mat4>,
) -> Result<()> {
    let first = &maps[0];
    let entities = source
        .base
        .entities
        .iter()
        .filter(|entity| {
            first.entities.contains_key(&entity.id)
                && !entity
                    .parent
                    .is_some_and(|parent| first.entities.contains_key(&parent))
        })
        .map(|entity| {
            (
                ObjectTarget::Authored(entity.id),
                entity.parent,
                entity.transform.translation,
            )
        });
    let instances = source
        .instances
        .iter()
        .filter(|instance| {
            first.instances.contains_key(&instance.id)
                && !instance
                    .parent
                    .is_some_and(|parent| first.entities.contains_key(&parent))
        })
        .map(|instance| {
            (
                ObjectTarget::Prefab(instance.id),
                instance.parent,
                instance.translation,
            )
        });
    let mut translations = BTreeMap::new();
    for (target, parent, translation) in entities.chain(instances) {
        let parent = parent.map_or(Mat4::IDENTITY, |parent| world[&parent]);
        let local_step = checked_transform_inverse(parent)?.transform_vector3(step);
        for (index, map) in maps.iter().enumerate() {
            let position = Vec3::from(translation) + local_step * (index + 1) as f32;
            ensure!(
                position.is_finite(),
                "placement translation is out of range"
            );
            let target = match target {
                ObjectTarget::Authored(id) => ObjectTarget::Authored(map.entities[&id]),
                ObjectTarget::Prefab(id) => ObjectTarget::Prefab(map.instances[&id]),
            };
            translations.insert(target, position.to_array());
        }
    }
    for entity in &mut source.base.entities {
        if let Some(translation) = translations.get(&ObjectTarget::Authored(entity.id)) {
            entity.transform.translation = *translation;
        }
    }
    for instance in &mut source.instances {
        if let Some(translation) = translations.get(&ObjectTarget::Prefab(instance.id)) {
            instance.translation = *translation;
        }
    }
    Ok(())
}

fn check_displacement(
    original: &BTreeMap<EntityId, Mat4>,
    placed: &BTreeMap<EntityId, Mat4>,
    old: EntityId,
    new: EntityId,
    expected: DVec3,
) -> Result<()> {
    let origin = original
        .get(&old)
        .context("placement source root changed")?
        .w_axis
        .truncate()
        .as_dvec3();
    let position = placed
        .get(&new)
        .context("placement candidate root changed")?
        .w_axis
        .truncate()
        .as_dvec3();
    let actual = position - origin;
    // A finite declaration can still round away the step when added to a large prefab root.
    ensure!(
        actual.is_finite() && actual.distance(expected) <= expected.length() * 2e-5,
        "cannot preserve world placement interval for {old}; transform precision is insufficient"
    );
    Ok(())
}
