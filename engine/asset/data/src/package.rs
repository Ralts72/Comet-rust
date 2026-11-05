//! Program-template and distributable package contracts; no filesystem or build execution.
use anyhow::{Result, ensure};
use comet_core::{AssetId, GameId, validate_digest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PACKAGE_VERSION: u32 = 4;
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
            index: crate::product::INDEX_VERSION,
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
    /// Program entry points supply their own build metadata; libraries own only the protocol.
    pub fn for_build(build_id: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            build_id: build_id.into(),
            target: target.into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            protocols: ProtocolVersions::current(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_digest(&self.build_id)?;
        for (field, value) in [
            ("target", &self.target),
            ("os", &self.os),
            ("arch", &self.arch),
        ] {
            ensure!(
                !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control()),
                "invalid runtime identity {field}"
            );
        }
        ensure!(
            self.protocols == ProtocolVersions::current(),
            "runtime resource protocols do not match"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppTemplateManifest {
    pub version: u32,
    pub identity: RuntimeIdentity,
    pub profile: String,
    pub app: String,
    pub app_sha256: String,
    pub editor_sha256: String,
    pub app_icon_sha256: String,
    pub notices: BTreeMap<String, String>,
}
impl AppTemplateManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 4 && self.profile == "release",
            "unsupported App template"
        );
        self.identity.validate()?;
        validate_digest(&self.app_sha256)?;
        validate_digest(&self.editor_sha256)?;
        validate_digest(&self.app_icon_sha256)?;
        executable_name(&self.app)?;
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
    pub game_id: GameId,
    pub entry_scene: AssetId,
    pub scenes: Vec<AssetId>,
    pub executable: String,
    /// Low-level asset-tool packing may use an externally supplied binary. Editor exports always
    /// supply the validated template identity.
    pub runtime: Option<RuntimeIdentity>,
    pub game_icon_sha256: Option<String>,
}
impl PackageManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == PACKAGE_VERSION,
            "unsupported App package version {}",
            self.version
        );
        let scenes: BTreeSet<_> = self.scenes.iter().copied().collect();
        ensure!(
            scenes.len() == self.scenes.len() && scenes.contains(&self.entry_scene),
            "invalid package scene list"
        );
        executable_name(&self.executable)?;
        if let Some(hash) = &self.game_icon_sha256 {
            validate_digest(hash)?;
        }
        if let Some(runtime) = &self.runtime {
            runtime.validate()?;
        }
        Ok(())
    }
    /// The running program supplies its identity, separately from manifest structure checks.
    pub fn validate_for(&self, expected: &RuntimeIdentity) -> Result<()> {
        self.validate()?;
        if let Some(runtime) = &self.runtime {
            ensure!(
                runtime == expected,
                "package and App build identities do not match"
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
