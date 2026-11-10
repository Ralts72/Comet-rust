use crate::EntityRef;
use anyhow::{Result, ensure};
use bevy_ecs::prelude::Component;
use comet_core::{AssetId, EntityId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

pub const MAX_ANIMATOR_BINDINGS: usize = 2048;
/// Keys identify source nodes; only the referenced scene entities are remapped by Prefab expansion.
pub type AnimatorBindings = BTreeMap<EntityId, Option<EntityRef>>;

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animator {
    pub skeleton: AssetId,
    pub clip: Option<AssetId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<AssetId>,
    #[serde(deserialize_with = "deserialize_bindings")]
    pub bindings: AnimatorBindings,
    pub looping: bool,
    pub speed: f32,
}
impl Animator {
    pub fn new(skeleton: AssetId) -> Self {
        Self {
            skeleton,
            clip: None,
            controller: None,
            bindings: BTreeMap::new(),
            looping: true,
            speed: 1.,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.clip.is_none() || self.controller.is_none(),
            "choose a clip or an animation controller, not both"
        );
        ensure!(
            self.bindings.len() <= MAX_ANIMATOR_BINDINGS,
            "animator binding budget exceeded"
        );
        ensure!(
            self.speed.is_finite() && self.speed >= 0.,
            "animation speed must be finite and nonnegative"
        );
        Ok(())
    }
}

/// A visual container follows a capsule's feet and supplies its controller's locomotion parameters.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterAnimation {
    pub character: Option<EntityRef>,
    pub animator: Option<EntityRef>,
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinBinding {
    pub skin: AssetId,
    pub animator: Option<EntityRef>,
}

/// Only missing or stale author bindings use this error; malformed assets and poses remain fatal.
#[derive(Clone, Debug)]
pub struct UnboundRig {
    pub entities: Vec<EntityId>,
}
impl fmt::Display for UnboundRig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "animation rig has missing or stale bindings on {} entities",
            self.entities.len()
        )?;
        for entity in self.entities.iter().take(8) {
            write!(f, " {entity}")?;
        }
        Ok(())
    }
}
impl std::error::Error for UnboundRig {}

pub(crate) fn deserialize_animator<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> std::result::Result<Option<Animator>, D::Error> {
    Animator::deserialize(value).map(Some)
}
pub(crate) fn deserialize_skin<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> std::result::Result<Option<SkinBinding>, D::Error> {
    SkinBinding::deserialize(value).map(Some)
}

pub(crate) fn deserialize_bindings<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> std::result::Result<AnimatorBindings, D::Error> {
    struct Bindings;
    impl<'de> serde::de::Visitor<'de> for Bindings {
        type Value = AnimatorBindings;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "at most {MAX_ANIMATOR_BINDINGS} unique source node bindings"
            )
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            use serde::de::Error;
            let mut bindings = BTreeMap::new();
            while let Some(node) = map.next_key::<EntityId>()? {
                if bindings.len() == MAX_ANIMATOR_BINDINGS {
                    return Err(M::Error::custom("animator binding budget exceeded"));
                }
                if bindings.contains_key(&node) {
                    return Err(M::Error::custom("duplicate source node binding"));
                }
                bindings.insert(node, map.next_value()?);
            }
            Ok(bindings)
        }
    }
    value.deserialize_map(Bindings)
}
