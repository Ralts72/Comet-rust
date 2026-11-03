//! Program-template and distributable package contracts; no filesystem or build execution.
use anyhow::{Result, ensure};
use comet_core::{AssetId, validate_digest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PACKAGE_VERSION: u32 = 2;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersions {
    pub product: u32,
    pub index: u32,
    pub content: u32,
    pub scene: u32,
}
impl ProtocolVersions {
    pub fn current() -> Self {
        Self {
            product: crate::product::PRODUCT_VERSION,
            index: 1,
            content: 1,
            scene: comet_scene::scene::SCENE_VERSION,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIdentity {
    pub build_id: String,
    pub target: String,
    pub os: String,
    pub arch: String,
    pub protocols: ProtocolVersions,
}
impl RuntimeIdentity {
    pub fn current() -> Self {
        Self {
            build_id: comet_core::BUILD_ID.into(),
            target: comet_core::BUILD_TARGET.into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            protocols: ProtocolVersions::current(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerTemplateManifest {
    pub version: u32,
    pub identity: RuntimeIdentity,
    pub profile: String,
    pub player: String,
    pub player_sha256: String,
    pub editor_sha256: String,
    pub app_icon_sha256: String,
    pub notices: BTreeMap<String, String>,
}
impl PlayerTemplateManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 3 && self.profile == "release",
            "unsupported Player template"
        );
        validate_digest(&self.identity.build_id)?;
        validate_digest(&self.player_sha256)?;
        validate_digest(&self.editor_sha256)?;
        validate_digest(&self.app_icon_sha256)?;
        ensure!(
            self.identity.protocols == ProtocolVersions::current(),
            "Player template resource protocols do not match"
        );
        executable_name(&self.player)?;
        for (name, hash) in &self.notices {
            executable_name(name)?;
            validate_digest(hash)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub version: u32,
    pub entry_scene: AssetId,
    pub scenes: Vec<AssetId>,
    pub executable: String,
    /// Low-level asset-tool packing may use an externally supplied binary. Editor exports always
    /// supply the validated template identity.
    pub runtime: Option<RuntimeIdentity>,
}
impl PackageManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == PACKAGE_VERSION,
            "unsupported Player package version {}",
            self.version
        );
        let scenes: BTreeSet<_> = self.scenes.iter().copied().collect();
        ensure!(
            scenes.len() == self.scenes.len() && scenes.contains(&self.entry_scene),
            "invalid package scene list"
        );
        executable_name(&self.executable)?;
        if let Some(runtime) = &self.runtime {
            ensure!(
                *runtime == RuntimeIdentity::current(),
                "package and Player build identities do not match"
            );
        }
        Ok(())
    }
}
fn executable_name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', ':']),
        "invalid package executable name"
    );
    Ok(())
}
