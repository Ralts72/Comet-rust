//! IDs are assigned by the protocol, never derived from Rust names.
//! Editing, prefab overrides, validation and reference traversal share this table.
use crate::{EntityData, EntityRef};
use anyhow::{Result, bail, ensure};
use comet_core::{AssetId, EntityId, InstanceId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
mod joints;
mod navigation;
pub use joints::*;
pub use navigation::*;
mod audio;
pub use audio::*;
mod animation;
pub use animation::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldKey {
    pub component: u32,
    pub field: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FieldValue {
    Vec3([f32; 3]),
    Quat([f32; 4]),
    Scalar(f32),
    Asset(AssetId),
    OptionalAsset(Option<AssetId>),
    Entity(Option<EntityRef>),
    Entities(Vec<EntityRef>),
    Bool(bool),
    ScriptParameters(crate::ScriptParameters),
    JointConnection(crate::JointConnection),
    JointLimits(Option<crate::JointLimits>),
    JointMotor(Option<crate::JointMotor>),
    AnimatorBindings(
        #[serde(deserialize_with = "crate::animation::deserialize_bindings")]
        crate::AnimatorBindings,
    ),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferencePolicy {
    None,
    Asset,
    Entity,
}
pub struct FieldDescriptor {
    pub key: FieldKey,
    pub path: &'static str,
    pub references: ReferencePolicy,
    read: fn(&EntityData) -> Option<FieldValue>,
    write: fn(&mut EntityData, FieldValue) -> Result<()>,
    validate: fn(&FieldValue) -> Result<()>,
}
pub const TRANSLATION: FieldKey = FieldKey {
    component: 1,
    field: 1,
};
pub const ROTATION: FieldKey = FieldKey {
    component: 1,
    field: 2,
};
pub const SCALE: FieldKey = FieldKey {
    component: 1,
    field: 3,
};
pub const MESH: FieldKey = FieldKey {
    component: 2,
    field: 1,
};
pub const MATERIAL: FieldKey = FieldKey {
    component: 2,
    field: 2,
};
pub const ROUGHNESS: FieldKey = FieldKey {
    component: 2,
    field: 3,
};
pub const TARGET: FieldKey = FieldKey {
    component: 3,
    field: 1,
};
pub const CONNECTIONS: FieldKey = FieldKey {
    component: 3,
    field: 2,
};
// Component 4: body configuration; component 5: character configuration.
pub const MASS: FieldKey = FieldKey {
    component: 4,
    field: 1,
};
pub const FRICTION: FieldKey = FieldKey {
    component: 4,
    field: 2,
};
pub const MOVE_SPEED: FieldKey = FieldKey {
    component: 5,
    field: 1,
};
pub const JUMP_SPEED: FieldKey = FieldKey {
    component: 5,
    field: 2,
};
pub const CROUCH_SPEED: FieldKey = FieldKey {
    component: 5,
    field: 3,
};
pub const CROUCH_HEIGHT_RATIO: FieldKey = FieldKey {
    component: 5,
    field: 4,
};
pub const SCRIPT_ASSET: FieldKey = FieldKey {
    component: 6,
    field: 1,
};
pub const SCRIPT_ENABLED: FieldKey = FieldKey {
    component: 6,
    field: 2,
};
pub const SCRIPT_PARAMETERS: FieldKey = FieldKey {
    component: 6,
    field: 3,
};
/// Minimum scene protocol that can describe a field or a prefab override.
pub fn required_scene_version(key: FieldKey) -> u32 {
    match key {
        FieldKey {
            component: 13..=14, ..
        } => 9,
        AUDIO_SPATIAL | AUDIO_MIN_DISTANCE | AUDIO_MAX_DISTANCE => 8,
        FieldKey { component: 12, .. } => 7,
        FieldKey {
            component: 10..=11, ..
        } => 6,
        FieldKey {
            component: 7..=9, ..
        } => 5,
        SCRIPT_ASSET | SCRIPT_ENABLED | SCRIPT_PARAMETERS => 4,
        CROUCH_SPEED | CROUCH_HEIGHT_RATIO => 3,
        MASS | FRICTION | MOVE_SPEED | JUMP_SPEED => 2,
        _ => 1,
    }
}
fn nonnegative_scalar(v: &FieldValue) -> Result<()> {
    ensure!(
        matches!(v,FieldValue::Scalar(x) if x.is_finite() && *x>=0.),
        "expected nonnegative finite scalar"
    );
    Ok(())
}
pub(crate) fn validate_pose(translation: [f32; 3], rotation: [f32; 4]) -> Result<()> {
    vector(&FieldValue::Vec3(translation))?;
    quaternion(&FieldValue::Quat(rotation))
}
fn vector(v: &FieldValue) -> Result<()> {
    ensure!(
        matches!(v,FieldValue::Vec3(v) if v.iter().all(|x|x.is_finite())),
        "expected finite vec3"
    );
    Ok(())
}
fn quaternion(v: &FieldValue) -> Result<()> {
    ensure!(
        matches!(v,FieldValue::Quat(v) if v.iter().all(|x|x.is_finite()) && (v.iter().map(|x|x*x).sum::<f32>()-1.).abs()<0.001),
        "expected normalized finite quaternion"
    );
    Ok(())
}
fn scalar(v: &FieldValue) -> Result<()> {
    ensure!(
        matches!(v,FieldValue::Scalar(v) if v.is_finite() && (0. ..=1.).contains(v)),
        "expected scalar in [0, 1]"
    );
    Ok(())
}
fn asset(v: &FieldValue) -> Result<()> {
    ensure!(
        matches!(v, FieldValue::Asset(_)),
        "expected asset reference"
    );
    Ok(())
}
fn entity(v: &FieldValue) -> Result<()> {
    ensure!(
        matches!(v, FieldValue::Entity(_)),
        "expected entity reference"
    );
    Ok(())
}
fn entities(v: &FieldValue) -> Result<()> {
    ensure!(
        matches!(v, FieldValue::Entities(_)),
        "expected entity references"
    );
    Ok(())
}
macro_rules! transform_field {
    ($key:ident,$field:ident,$variant:ident,$validate:ident) => {
        FieldDescriptor {
            key: $key,
            path: concat!("transform.", stringify!($field)),
            references: ReferencePolicy::None,
            read: |e| Some(FieldValue::$variant(e.transform.$field)),
            write: |e, v| {
                if let FieldValue::$variant(v) = v {
                    e.transform.$field = v;
                    Ok(())
                } else {
                    bail!("field type mismatch")
                }
            },
            validate: $validate,
        }
    };
}
static FIELDS: &[FieldDescriptor] = &[
    FieldDescriptor {
        key: SCRIPT_ASSET,
        path: "script.asset",
        references: ReferencePolicy::Asset,
        read: |e| {
            e.script
                .as_ref()
                .map(|script| FieldValue::Asset(script.asset))
        },
        write: |e, value| {
            if let (Some(script), FieldValue::Asset(asset)) = (&mut e.script, value) {
                script.asset = asset;
                Ok(())
            } else {
                bail!("missing script or wrong asset value")
            }
        },
        validate: asset,
    },
    FieldDescriptor {
        key: SCRIPT_ENABLED,
        path: "script.enabled",
        references: ReferencePolicy::None,
        read: |e| {
            e.script
                .as_ref()
                .map(|script| FieldValue::Bool(script.enabled))
        },
        write: |e, value| {
            if let (Some(script), FieldValue::Bool(enabled)) = (&mut e.script, value) {
                script.enabled = enabled;
                Ok(())
            } else {
                bail!("missing script or wrong bool value")
            }
        },
        validate: |value| {
            ensure!(matches!(value, FieldValue::Bool(_)), "expected bool");
            Ok(())
        },
    },
    FieldDescriptor {
        key: SCRIPT_PARAMETERS,
        path: "script.parameters",
        references: ReferencePolicy::Entity,
        read: |e| {
            e.script
                .as_ref()
                .map(|script| FieldValue::ScriptParameters(script.parameters.clone()))
        },
        write: |e, value| {
            if let (Some(script), FieldValue::ScriptParameters(parameters)) = (&mut e.script, value)
            {
                script.parameters = parameters;
                Ok(())
            } else {
                bail!("missing script or wrong parameter value")
            }
        },
        validate: |value| {
            if let FieldValue::ScriptParameters(parameters) = value {
                crate::script::validate_parameters(parameters)
            } else {
                bail!("expected script parameters")
            }
        },
    },
    transform_field!(TRANSLATION, translation, Vec3, vector),
    transform_field!(ROTATION, rotation, Quat, quaternion),
    transform_field!(SCALE, scale, Vec3, vector),
    FieldDescriptor {
        key: MESH,
        path: "model.mesh",
        references: ReferencePolicy::Asset,
        read: |e| e.model.as_ref().map(|m| FieldValue::Asset(m.mesh)),
        write: |e, v| {
            if let (Some(m), FieldValue::Asset(v)) = (&mut e.model, v) {
                m.mesh = v;
                Ok(())
            } else {
                bail!("missing model or wrong value")
            }
        },
        validate: asset,
    },
    FieldDescriptor {
        key: MATERIAL,
        path: "model.surface.material",
        references: ReferencePolicy::Asset,
        read: |e| {
            e.model
                .as_ref()
                .map(|m| FieldValue::Asset(m.surface.material))
        },
        write: |e, v| {
            if let (Some(m), FieldValue::Asset(v)) = (&mut e.model, v) {
                m.surface.material = v;
                Ok(())
            } else {
                bail!("missing model or wrong value")
            }
        },
        validate: asset,
    },
    FieldDescriptor {
        key: ROUGHNESS,
        path: "model.surface.roughness",
        references: ReferencePolicy::None,
        read: |e| {
            e.model
                .as_ref()
                .map(|m| FieldValue::Scalar(m.surface.roughness))
        },
        write: |e, v| {
            if let (Some(m), FieldValue::Scalar(v)) = (&mut e.model, v) {
                m.surface.roughness = v;
                Ok(())
            } else {
                bail!("missing model or wrong value")
            }
        },
        validate: scalar,
    },
    FieldDescriptor {
        key: TARGET,
        path: "links.target",
        references: ReferencePolicy::Entity,
        read: |e| Some(FieldValue::Entity(e.links.target.clone())),
        write: |e, v| {
            if let FieldValue::Entity(v) = v {
                e.links.target = v;
                Ok(())
            } else {
                bail!("wrong value")
            }
        },
        validate: entity,
    },
    FieldDescriptor {
        key: CONNECTIONS,
        path: "links.connections",
        references: ReferencePolicy::Entity,
        read: |e| Some(FieldValue::Entities(e.links.connections.clone())),
        write: |e, v| {
            if let FieldValue::Entities(v) = v {
                e.links.connections = v;
                Ok(())
            } else {
                bail!("wrong value")
            }
        },
        validate: entities,
    },
    FieldDescriptor {
        key: MASS,
        path: "physics.mass",
        references: ReferencePolicy::None,
        read: |e| e.physics.as_ref().map(|c| FieldValue::Scalar(c.mass)),
        write: |e, v| {
            if let (Some(c), FieldValue::Scalar(v)) = (&mut e.physics, v) {
                c.mass = v;
                Ok(())
            } else {
                bail!("missing component or wrong value")
            }
        },
        validate: nonnegative_scalar,
    },
    FieldDescriptor {
        key: FRICTION,
        path: "physics.friction",
        references: ReferencePolicy::None,
        read: |e| e.physics.as_ref().map(|c| FieldValue::Scalar(c.friction)),
        write: |e, v| {
            if let (Some(c), FieldValue::Scalar(v)) = (&mut e.physics, v) {
                c.friction = v;
                Ok(())
            } else {
                bail!("missing component or wrong value")
            }
        },
        validate: nonnegative_scalar,
    },
    FieldDescriptor {
        key: MOVE_SPEED,
        path: "character.move_speed",
        references: ReferencePolicy::None,
        read: |e| {
            e.character
                .as_ref()
                .map(|c| FieldValue::Scalar(c.move_speed))
        },
        write: |e, v| {
            if let (Some(c), FieldValue::Scalar(v)) = (&mut e.character, v) {
                c.move_speed = v;
                Ok(())
            } else {
                bail!("missing component or wrong value")
            }
        },
        validate: nonnegative_scalar,
    },
    FieldDescriptor {
        key: JUMP_SPEED,
        path: "character.jump_speed",
        references: ReferencePolicy::None,
        read: |e| {
            e.character
                .as_ref()
                .map(|c| FieldValue::Scalar(c.jump_speed))
        },
        write: |e, v| {
            if let (Some(c), FieldValue::Scalar(v)) = (&mut e.character, v) {
                c.jump_speed = v;
                Ok(())
            } else {
                bail!("missing component or wrong value")
            }
        },
        validate: nonnegative_scalar,
    },
    FieldDescriptor {
        key: CROUCH_SPEED,
        path: "character.crouch_speed",
        references: ReferencePolicy::None,
        read: |e| {
            e.character
                .as_ref()
                .map(|c| FieldValue::Scalar(c.crouch_speed))
        },
        write: |e, v| {
            if let (Some(c), FieldValue::Scalar(v)) = (&mut e.character, v) {
                c.crouch_speed = v;
                Ok(())
            } else {
                bail!("missing component or wrong value")
            }
        },
        validate: nonnegative_scalar,
    },
    FieldDescriptor {
        key: CROUCH_HEIGHT_RATIO,
        path: "character.crouch_height_ratio",
        references: ReferencePolicy::None,
        read: |e| {
            e.character
                .as_ref()
                .map(|c| FieldValue::Scalar(c.crouch_height_ratio))
        },
        write: |e, v| {
            if let (Some(c), FieldValue::Scalar(v)) = (&mut e.character, v) {
                c.crouch_height_ratio = v;
                Ok(())
            } else {
                bail!("missing component or wrong value")
            }
        },
        validate: |v| {
            ensure!(
                matches!(v, FieldValue::Scalar(v) if v.is_finite() && (0.1..=1.).contains(v)),
                "expected crouch height ratio in [0.1, 1]"
            );
            Ok(())
        },
    },
];
fn fields() -> impl Iterator<Item = &'static FieldDescriptor> {
    FIELDS
        .iter()
        .chain(joints::FIELDS)
        .chain(navigation::FIELDS)
        .chain(audio::FIELDS)
        .chain(animation::FIELDS)
}
pub fn descriptor(key: FieldKey) -> Result<&'static FieldDescriptor> {
    fields()
        .find(|f| f.key == key)
        .ok_or_else(|| anyhow::anyhow!("unknown field {}/{}", key.component, key.field))
}
impl FieldDescriptor {
    pub fn read(&self, e: &EntityData) -> Option<FieldValue> {
        (self.read)(e)
    }
    pub fn validate_value(&self, value: &FieldValue) -> Result<()> {
        (self.validate)(value)
    }
    pub fn write(&self, e: &mut EntityData, v: FieldValue) -> Result<()> {
        self.validate_value(&v)?;
        (self.write)(e, v)
    }
}
pub fn validate(e: &EntityData) -> Result<()> {
    if let Some(animator) = &e.animator {
        animator.validate()?;
    }
    ensure!(
        e.skin.is_none() || e.model.is_some(),
        "skin binding requires a model"
    );
    crate::physics::validate(e)?;
    crate::joints::validate(e)?;
    crate::navigation::validate(e)?;
    if let Some(source) = &e.audio_source {
        source
            .validate()
            .map_err(|error| anyhow::anyhow!("entity {} audio source: {error}", e.id))?;
    }
    for f in fields() {
        if let Some(v) = f.read(e) {
            (f.validate)(&v)
                .map_err(|err| anyhow::anyhow!("entity {} field {}: {err}", e.id, f.path))?;
        }
    }
    Ok(())
}
/// References stored inside one schema value, without treating source-node keys as targets.
pub fn value_entity_references(value: &FieldValue) -> Vec<&EntityRef> {
    match value {
        FieldValue::Entity(reference) => reference.iter().collect(),
        FieldValue::Entities(references) => references.iter().collect(),
        FieldValue::AnimatorBindings(bindings) => bindings.values().flatten().collect(),
        FieldValue::JointConnection(crate::JointConnection::Body(reference)) => vec![reference],
        FieldValue::ScriptParameters(parameters) => parameters
            .values()
            .filter_map(|value| {
                if let crate::ScriptValue::Entity(Some(reference)) = value {
                    Some(reference)
                } else {
                    None
                }
            })
            .collect(),
        _ => vec![],
    }
}
/// A field's old component version alone cannot describe a newer reference inside its value.
pub fn required_value_scene_version(value: &FieldValue) -> u32 {
    value_entity_references(value)
        .into_iter()
        .map(EntityRef::required_scene_version)
        .max()
        .unwrap_or(1)
}
pub fn entity_references(e: &EntityData) -> Vec<EntityRef> {
    fields()
        .filter(|field| field.references == ReferencePolicy::Entity)
        .filter_map(|field| field.read(e))
        .flat_map(|value| {
            value_entity_references(&value)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect()
}
pub fn asset_references(e: &EntityData) -> BTreeSet<AssetId> {
    let mut result: BTreeSet<_> = fields()
        .filter(|f| f.references == ReferencePolicy::Asset)
        .filter_map(|f| match f.read(e) {
            Some(FieldValue::Asset(id)) => Some(id),
            Some(FieldValue::OptionalAsset(id)) => id,
            _ => None,
        })
        .collect();
    for r in entity_references(e) {
        if let EntityRef::External { scene, .. } = r {
            result.insert(scene);
        }
    }
    result
}
pub fn remap(e: &mut EntityData, map: &BTreeMap<EntityId, EntityId>) -> Result<()> {
    map_entity_references(e, |reference| {
        if let EntityRef::Local { entity } = reference
            && let Some(id) = map.get(entity)
        {
            *entity = *id;
        }
        Ok(())
    })
}
/// Only the declaration identity changes; the referenced prefab's source UUID is a separate namespace.
pub fn remap_instance_references(
    e: &mut EntityData,
    map: &BTreeMap<InstanceId, InstanceId>,
) -> Result<()> {
    map_entity_references(e, |reference| {
        if let EntityRef::InInstance { instance, .. } = reference
            && let Some(id) = map.get(instance)
        {
            *instance = *id;
        }
        Ok(())
    })
}
/// Rewrite reference values without changing source-node keys or collection structure.
pub fn map_value_entity_references(
    value: &mut FieldValue,
    mut visit: impl FnMut(&mut EntityRef) -> Result<()>,
) -> Result<()> {
    match value {
        FieldValue::Entity(Some(reference)) => visit(reference)?,
        FieldValue::JointConnection(crate::JointConnection::Body(reference)) => visit(reference)?,
        FieldValue::Entities(references) => {
            for reference in references {
                visit(reference)?;
            }
        }
        FieldValue::AnimatorBindings(bindings) => {
            for reference in bindings.values_mut().flatten() {
                visit(reference)?;
            }
        }
        FieldValue::ScriptParameters(parameters) => {
            for value in parameters.values_mut() {
                if let crate::ScriptValue::Entity(Some(reference)) = value {
                    visit(reference)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn map_entity_references(
    e: &mut EntityData,
    mut visit: impl FnMut(&mut EntityRef) -> Result<()>,
) -> Result<()> {
    for field in fields().filter(|field| field.references == ReferencePolicy::Entity) {
        if let Some(mut value) = field.read(e) {
            map_value_entity_references(&mut value, &mut visit)?;
            field.write(e, value)?;
        }
    }
    Ok(())
}

/// Deleting local objects clears local links, while external scene identities remain unchanged.
pub(crate) fn clear_removed_references(
    e: &mut EntityData,
    removed: &BTreeSet<EntityId>,
) -> Result<()> {
    clear_references(
        e,
        |reference| !matches!(reference, EntityRef::Local { entity } if removed.contains(entity)),
    )
}
/// Explicit instance removal clears references owned by this source, never a child override's scope.
pub fn clear_instance_references(e: &mut EntityData, removed: &BTreeSet<InstanceId>) -> Result<()> {
    clear_references(
        e,
        |reference| !matches!(reference, EntityRef::InInstance { instance, .. } if removed.contains(instance)),
    )
}
fn clear_references(e: &mut EntityData, keep: impl Fn(&EntityRef) -> bool) -> Result<()> {
    for field in fields().filter(|field| field.references == ReferencePolicy::Entity) {
        if let Some(mut value) = field.read(e) {
            match &mut value {
                FieldValue::Entity(reference) => {
                    if reference.as_ref().is_some_and(|r| !keep(r)) {
                        *reference = None;
                    }
                }
                FieldValue::Entities(references) => references.retain(&keep),
                FieldValue::AnimatorBindings(bindings) => {
                    for r in bindings.values_mut() {
                        if r.as_ref().is_some_and(|r| !keep(r)) {
                            *r = None;
                        }
                    }
                }
                FieldValue::JointConnection(connection) => {
                    if let crate::JointConnection::Body(reference) = connection
                        && !keep(reference)
                    {
                        *connection = crate::JointConnection::Unbound;
                    }
                }
                FieldValue::ScriptParameters(parameters) => {
                    for value in parameters.values_mut() {
                        if let crate::ScriptValue::Entity(reference) = value
                            && reference.as_ref().is_some_and(|r| !keep(r))
                        {
                            *reference = None;
                        }
                    }
                }
                _ => {}
            }
            field.write(e, value)?;
        }
    }
    Ok(())
}
