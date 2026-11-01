use bevy_ecs::prelude::Component;
use comet_core::{AssetId, EntityId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntityRef {
    Local { entity: EntityId },
    External { scene: AssetId, entity: EntityId },
}
impl EntityRef {
    pub fn local(entity: EntityId) -> Self {
        Self::Local { entity }
    }
}
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Quaternion in xyzw order.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            scale: [1.; 3],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub material: AssetId,
    pub roughness: f32,
}
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub mesh: AssetId,
    pub surface: Surface,
}
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Links {
    pub target: Option<EntityRef>,
    pub connections: Vec<EntityRef>,
}
/// Serialized authoring data; ECS stores the concrete components separately.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityData {
    pub id: EntityId,
    pub name: String,
    pub parent: Option<EntityId>,
    pub transform: Transform,
    pub model: Option<Model>,
    pub links: Links,
}
impl EntityData {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            parent: None,
            transform: Transform::default(),
            model: None,
            links: Links::default(),
        }
    }
}
#[derive(Component)]
pub(crate) struct Identity {
    pub id: EntityId,
    pub name: String,
    pub parent: Option<EntityId>,
}
