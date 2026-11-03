//! IDs are assigned by the protocol, never derived from Rust names.
//! Editing, prefab overrides, validation and reference traversal share this table.
use crate::{EntityData, EntityRef};
use anyhow::{Result, bail, ensure};
use comet_core::{AssetId, EntityId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    Entity(Option<EntityRef>),
    Entities(Vec<EntityRef>),
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
pub static FIELDS: &[FieldDescriptor] = &[
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
];
pub fn descriptor(key: FieldKey) -> Result<&'static FieldDescriptor> {
    FIELDS
        .iter()
        .find(|f| f.key == key)
        .ok_or_else(|| anyhow::anyhow!("unknown field {}/{}", key.component, key.field))
}
impl FieldDescriptor {
    pub fn read(&self, e: &EntityData) -> Option<FieldValue> {
        (self.read)(e)
    }
    pub fn write(&self, e: &mut EntityData, v: FieldValue) -> Result<()> {
        (self.validate)(&v)?;
        (self.write)(e, v)
    }
}
pub fn validate(e: &EntityData) -> Result<()> {
    crate::physics::validate(e)?;
    for f in FIELDS {
        if let Some(v) = f.read(e) {
            (f.validate)(&v)
                .map_err(|err| anyhow::anyhow!("entity {} field {}: {err}", e.id, f.path))?;
        }
    }
    Ok(())
}
pub fn entity_references(e: &EntityData) -> Vec<EntityRef> {
    FIELDS
        .iter()
        .filter(|f| f.references == ReferencePolicy::Entity)
        .flat_map(|f| match f.read(e) {
            Some(FieldValue::Entity(v)) => v.into_iter().collect(),
            Some(FieldValue::Entities(v)) => v,
            _ => vec![],
        })
        .collect()
}
pub fn asset_references(e: &EntityData) -> BTreeSet<AssetId> {
    let mut result: BTreeSet<_> = FIELDS
        .iter()
        .filter(|f| f.references == ReferencePolicy::Asset)
        .filter_map(|f| match f.read(e) {
            Some(FieldValue::Asset(id)) => Some(id),
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
    fn reference(r: &mut EntityRef, map: &BTreeMap<EntityId, EntityId>) {
        if let EntityRef::Local { entity } = r
            && let Some(id) = map.get(entity)
        {
            *entity = *id;
        }
    }
    for f in FIELDS
        .iter()
        .filter(|f| f.references == ReferencePolicy::Entity)
    {
        if let Some(mut v) = f.read(e) {
            match &mut v {
                FieldValue::Entity(Some(r)) => reference(r, map),
                FieldValue::Entities(rs) => {
                    for r in rs {
                        reference(r, map);
                    }
                }
                _ => {}
            }
            f.write(e, v)?;
        }
    }
    Ok(())
}
