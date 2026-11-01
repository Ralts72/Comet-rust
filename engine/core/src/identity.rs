use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

macro_rules! identity {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
        )]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl TryFrom<Uuid> for $name {
            type Error = anyhow::Error;
            fn try_from(value: Uuid) -> Result<Self> {
                ensure!(
                    !value.is_nil(),
                    concat!(stringify!($name), " cannot be nil")
                );
                Ok(Self(value))
            }
        }
        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
        impl FromStr for $name {
            type Err = anyhow::Error;
            fn from_str(value: &str) -> Result<Self> {
                Self::try_from(Uuid::parse_str(value)?)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
identity!(EntityId);
identity!(AssetId);
identity!(InstanceId);
identity!(WorldId);

impl EntityId {
    pub fn in_asset(asset: AssetId, key: &str) -> Self {
        Self(Uuid::new_v5(&asset.0, key.as_bytes()))
    }
    /// Stable across rebuilding the same instance from updated authoring content.
    pub fn in_instance(self, instance: InstanceId) -> Self {
        Self(Uuid::new_v5(&instance.0, self.0.as_bytes()))
    }
}
impl AssetId {
    pub fn child(self, key: &str) -> Self {
        Self(Uuid::new_v5(&self.0, key.as_bytes()))
    }
}
impl InstanceId {
    pub fn in_instance(self, parent: InstanceId) -> Self {
        Self(Uuid::new_v5(&parent.0, self.0.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_instance_identity_and_nil_rejection() {
        let entity = EntityId::new();
        let instance = InstanceId::new();
        assert_eq!(entity.in_instance(instance), entity.in_instance(instance));
        assert_ne!(
            entity.in_instance(instance),
            entity.in_instance(InstanceId::new())
        );
        assert!(
            "00000000-0000-0000-0000-000000000000"
                .parse::<AssetId>()
                .is_err()
        );
    }
}
