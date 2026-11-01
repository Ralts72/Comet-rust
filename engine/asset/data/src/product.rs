//! Versioned, CPU-only products. No importer or GPU handles appear in this protocol.
use anyhow::{Context, Result, ensure};
use comet_core::{AssetId, validate_digest};
use comet_scene::{ContentSource, EntityRef, schema};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCT_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Scene,
    Prefab,
    Mesh,
    Material,
    Texture,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshData {
    pub version: u32,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}
impl MeshData {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported mesh version");
        ensure!(!self.positions.is_empty(), "empty mesh");
        ensure!(
            self.positions.len() == self.normals.len() && self.positions.len() == self.uvs.len(),
            "mesh attribute count mismatch"
        );
        ensure!(
            !self.indices.is_empty() && self.indices.len().is_multiple_of(3),
            "expected triangle indices"
        );
        ensure!(
            self.indices
                .iter()
                .all(|i| (*i as usize) < self.positions.len()),
            "mesh index out of bounds"
        );
        ensure!(
            self.positions
                .iter()
                .chain(&self.normals)
                .flatten()
                .chain(self.uvs.iter().flatten())
                .all(|x| x.is_finite()),
            "non-finite mesh attribute"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialData {
    pub version: u32,
    pub base_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub base_color_texture: Option<AssetId>,
}
impl MaterialData {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported material version");
        ensure!(
            self.base_color
                .iter()
                .chain([&self.roughness, &self.metallic])
                .all(|x| x.is_finite() && (0. ..=1.).contains(x)),
            "material values must be finite and in [0, 1]"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}
impl TextureData {
    pub fn validate(&self) -> Result<()> {
        let size = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|s| s.checked_mul(4))
            .context("texture dimensions overflow")?;
        ensure!(
            self.width > 0 && self.height > 0 && size == self.rgba8.len(),
            "invalid RGBA8 texture dimensions/byte count"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Product {
    Scene(ContentSource),
    Prefab(ContentSource),
    Mesh(MeshData),
    Material(MaterialData),
    Texture(TextureData),
}
impl Product {
    pub fn kind(&self) -> AssetKind {
        match self {
            Self::Scene(_) => AssetKind::Scene,
            Self::Prefab(_) => AssetKind::Prefab,
            Self::Mesh(_) => AssetKind::Mesh,
            Self::Material(_) => AssetKind::Material,
            Self::Texture(_) => AssetKind::Texture,
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Scene(s) | Self::Prefab(s) => s.validate(),
            Self::Mesh(m) => m.validate(),
            Self::Material(m) => m.validate(),
            Self::Texture(t) => t.validate(),
        }
    }
    pub fn dependencies(&self) -> BTreeSet<AssetId> {
        match self {
            Self::Scene(s) | Self::Prefab(s) => s.dependencies(),
            Self::Material(m) => m.base_color_texture.into_iter().collect(),
            _ => BTreeSet::new(),
        }
    }
    pub fn validate_dependency_kinds(&self, kinds: &BTreeMap<AssetId, AssetKind>) -> Result<()> {
        let expect = |id: AssetId, kind: AssetKind| -> Result<()> {
            ensure!(
                kinds.get(&id) == Some(&kind),
                "asset {id} expected {kind:?}, found {:?}",
                kinds.get(&id)
            );
            Ok(())
        };
        for id in self.dependencies() {
            ensure!(kinds.contains_key(&id), "missing dependency {id}");
        }
        match self {
            Self::Scene(s) | Self::Prefab(s) => {
                for e in &s.base.entities {
                    if let Some(m) = &e.model {
                        expect(m.mesh, AssetKind::Mesh)?;
                        expect(m.surface.material, AssetKind::Material)?;
                    }
                    for r in schema::entity_references(e) {
                        if let EntityRef::External { scene, .. } = r {
                            expect(scene, AssetKind::Scene)?;
                        }
                    }
                }
                for i in &s.instances {
                    expect(i.prefab, AssetKind::Prefab)?;
                    for o in &i.overrides {
                        if let schema::FieldValue::Asset(id) = o.value {
                            if o.field == schema::MESH {
                                expect(id, AssetKind::Mesh)?;
                            }
                            if o.field == schema::MATERIAL {
                                expect(id, AssetKind::Material)?;
                            }
                        }
                    }
                }
            }
            Self::Material(m) => {
                if let Some(id) = m.base_color_texture {
                    expect(id, AssetKind::Texture)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductFile {
    pub version: u32,
    pub id: AssetId,
    pub source_digest: String,
    pub payload: Product,
}
impl ProductFile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == PRODUCT_VERSION,
            "unsupported product version {}",
            self.version
        );
        validate_digest(&self.source_digest)?;
        self.payload.validate()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    pub digest: String,
    pub kind: AssetKind,
    pub dependencies: BTreeSet<AssetId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductIndex {
    pub version: u32,
    pub products: BTreeMap<AssetId, IndexEntry>,
}
impl ProductIndex {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported index version {}",
            self.version
        );
        for (id, e) in &self.products {
            validate_digest(&e.digest)?;
            for dep in &e.dependencies {
                ensure!(
                    self.products.contains_key(dep),
                    "asset {id} missing dependency {dep}"
                );
            }
        }
        Ok(())
    }
}
