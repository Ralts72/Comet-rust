use bevy_ecs::prelude::Component;
use comet_core::{AssetId, EntityId, InstanceId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntityRef {
    Local {
        entity: EntityId,
    },
    External {
        scene: AssetId,
        entity: EntityId,
    },
    /// A source entity in a direct instance declared by the same ContentSource.
    InInstance {
        instance: InstanceId,
        entity: EntityId,
    },
}
impl EntityRef {
    pub fn local(entity: EntityId) -> Self {
        Self::Local { entity }
    }
    pub fn in_instance(instance: InstanceId, entity: EntityId) -> Self {
        Self::InInstance { instance, entity }
    }
    pub fn required_scene_version(&self) -> u32 {
        match self {
            Self::InInstance { .. } => 10,
            Self::Local { .. } | Self::External { .. } => 1,
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<crate::PhysicsBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<crate::Character>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<crate::Script>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_joint: Option<crate::PointJoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hinge_joint: Option<crate::HingeJoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slider_joint: Option<crate::SliderJoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_region: Option<crate::NavigationRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_agent: Option<crate::NavigationAgent>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::audio::deserialize_source"
    )]
    pub audio_source: Option<crate::AudioSource>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::animation::deserialize_animator"
    )]
    pub animator: Option<crate::Animator>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::animation::deserialize_skin"
    )]
    pub skin: Option<crate::SkinBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_animation: Option<crate::CharacterAnimation>,
}
impl EntityData {
    /// Minimum protocol for the actual component configuration and its reference values.
    pub fn required_scene_version(&self) -> u32 {
        let mut version = 1;
        if self.physics.is_some() || self.character.is_some() {
            version = 2;
        }
        if let Some(character) = &self.character {
            version = version.max(character.required_scene_version());
        }
        if self.script.is_some() {
            version = version.max(4);
        }
        if self.point_joint.is_some() || self.hinge_joint.is_some() || self.slider_joint.is_some() {
            version = version.max(5);
        }
        if self.navigation_region.is_some() || self.navigation_agent.is_some() {
            version = version.max(6);
        }
        if let Some(source) = &self.audio_source {
            version = version.max(source.required_scene_version());
        }
        if self.animator.is_some() || self.skin.is_some() {
            version = version.max(9);
        }
        if self
            .animator
            .as_ref()
            .is_some_and(|a| a.controller.is_some())
            || self.character_animation.is_some()
        {
            version = version.max(11);
        }
        crate::schema::entity_references(self)
            .iter()
            .map(EntityRef::required_scene_version)
            .fold(version, u32::max)
    }

    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            parent: None,
            transform: Transform::default(),
            model: None,
            links: Links::default(),
            physics: None,
            character: None,
            script: None,
            point_joint: None,
            hinge_joint: None,
            slider_joint: None,
            navigation_region: None,
            navigation_agent: None,
            audio_source: None,
            animator: None,
            skin: None,
            character_animation: None,
        }
    }
}
#[derive(Component)]
pub(crate) struct Identity {
    pub id: EntityId,
    pub name: String,
    pub parent: Option<EntityId>,
}
