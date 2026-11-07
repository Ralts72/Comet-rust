//! Versioned, CPU-only products. No importer or GPU handles appear in this protocol.
use crate::animation::{AnimationClipData, MAX_SKIN_VERTICES, SkeletonData, SkinData};
pub use crate::{audio::AudioData, navigation::NavigationData};
use anyhow::{Context, Result, ensure};
use comet_core::{AssetId, validate_digest};
use comet_scene::{ContentSource, EntityRef, SceneSnapshot, schema};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCT_VERSION: u32 = 5;
pub const INDEX_VERSION: u32 = 6;
pub const SCRIPT_VERSION: u32 = 1;
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;
pub const MAX_EDITOR_LABEL_BYTES: usize = 4 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Scene,
    Prefab,
    Mesh,
    Material,
    Texture,
    Script,
    Navigation,
    Audio,
    Skeleton,
    Skin,
    AnimationClip,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptData {
    pub version: u32,
    pub source: String,
}
impl ScriptData {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == SCRIPT_VERSION, "unsupported script version");
        ensure!(!self.source.trim().is_empty(), "script source is empty");
        ensure!(!self.source.contains('\0'), "script source contains NUL");
        ensure!(
            self.source.len() <= MAX_SCRIPT_BYTES,
            "script source exceeds 64 KiB"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshData {
    pub version: u32,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "non_null_attribute"
    )]
    pub joints: Option<Vec<[u16; 4]>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "non_null_attribute"
    )]
    pub weights: Option<Vec<[f32; 4]>>,
}
fn non_null_attribute<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
impl MeshData {
    pub fn validate(&self) -> Result<()> {
        ensure!(matches!(self.version, 1 | 2), "unsupported mesh version");
        ensure!(
            self.version == 2 || (self.joints.is_none() && self.weights.is_none()),
            "mesh v1 cannot contain skin attributes"
        );
        match (&self.joints, &self.weights) {
            (None, None) => {}
            (Some(joints), Some(weights)) => {
                ensure!(
                    self.positions.len() <= MAX_SKIN_VERTICES
                        && joints.len() == self.positions.len()
                        && weights.len() == self.positions.len(),
                    "skin vertex attribute count/budget mismatch"
                );
                for (joints, weights) in joints.iter().zip(weights) {
                    ensure!(
                        weights.iter().all(|v| v.is_finite() && *v >= 0.),
                        "invalid skin weight"
                    );
                    let sum: f64 = weights.iter().map(|v| f64::from(*v)).sum();
                    ensure!((sum - 1.).abs() <= 1e-5, "skin weights must be normalized");
                    let mut active = BTreeSet::new();
                    for (joint, weight) in joints.iter().zip(weights) {
                        ensure!(
                            *weight == 0. || active.insert(joint),
                            "duplicate active joint influence"
                        );
                    }
                }
            }
            _ => anyhow::bail!("skin joint/weight attributes must be paired"),
        }
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
    Script(ScriptData),
    Navigation(NavigationData),
    Audio(AudioData),
    Skeleton(SkeletonData),
    Skin(SkinData),
    AnimationClip(AnimationClipData),
}
impl Product {
    pub fn kind(&self) -> AssetKind {
        match self {
            Self::Scene(_) => AssetKind::Scene,
            Self::Prefab(_) => AssetKind::Prefab,
            Self::Mesh(_) => AssetKind::Mesh,
            Self::Material(_) => AssetKind::Material,
            Self::Texture(_) => AssetKind::Texture,
            Self::Script(_) => AssetKind::Script,
            Self::Navigation(_) => AssetKind::Navigation,
            Self::Audio(_) => AssetKind::Audio,
            Self::Skeleton(_) => AssetKind::Skeleton,
            Self::Skin(_) => AssetKind::Skin,
            Self::AnimationClip(_) => AssetKind::AnimationClip,
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Scene(s) | Self::Prefab(s) => s.validate(),
            Self::Mesh(m) => m.validate(),
            Self::Material(m) => m.validate(),
            Self::Texture(t) => t.validate(),
            Self::Script(s) => s.validate(),
            Self::Navigation(n) => n.validate(),
            Self::Audio(audio) => audio.validate(),
            Self::Skeleton(value) => value.validate(),
            Self::Skin(value) => value.validate(),
            Self::AnimationClip(value) => value.validate(),
        }
    }
    pub fn dependencies(&self) -> BTreeSet<AssetId> {
        match self {
            Self::Scene(s) | Self::Prefab(s) => s.dependencies(),
            Self::Material(m) => m.base_color_texture.into_iter().collect(),
            Self::Skin(value) => [value.skeleton].into_iter().collect(),
            Self::AnimationClip(value) => [value.skeleton].into_iter().collect(),
            _ => BTreeSet::new(),
        }
    }
    pub fn validate_dependency_kinds(
        &self,
        kind_of: impl Fn(AssetId) -> Option<AssetKind>,
    ) -> Result<()> {
        let expect = |id, kind| expect_kind(&kind_of, id, kind);
        for id in self.dependencies() {
            ensure!(kind_of(id).is_some(), "missing dependency {id}");
        }
        match self {
            Self::Scene(s) | Self::Prefab(s) => {
                validate_entity_dependency_kinds(&s.base, &kind_of)?;
                for i in &s.instances {
                    expect(i.prefab, AssetKind::Prefab)?;
                    for o in &i.overrides {
                        if let schema::FieldValue::OptionalAsset(Some(id)) = o.value {
                            if o.field == schema::NAVIGATION_DATA {
                                expect(id, AssetKind::Navigation)?;
                            }
                            if o.field == schema::ANIMATOR_CLIP {
                                expect(id, AssetKind::AnimationClip)?;
                            }
                        }
                        if let schema::FieldValue::Asset(id) = o.value {
                            if o.field == schema::MESH {
                                expect(id, AssetKind::Mesh)?;
                            }
                            if o.field == schema::MATERIAL {
                                expect(id, AssetKind::Material)?;
                            }
                            if o.field == schema::SCRIPT_ASSET {
                                expect(id, AssetKind::Script)?;
                            }
                            if o.field == schema::ANIMATOR_SKELETON {
                                expect(id, AssetKind::Skeleton)?;
                            }
                            if o.field == schema::SKIN_ASSET {
                                expect(id, AssetKind::Skin)?;
                            }
                            if o.field == schema::AUDIO_CLIP {
                                expect(id, AssetKind::Audio)?;
                            }
                        }
                    }
                }
            }
            Self::Skin(value) => expect(value.skeleton, AssetKind::Skeleton)?,
            Self::AnimationClip(value) => expect(value.skeleton, AssetKind::Skeleton)?,
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
/// Check a borrowed world without constructing a temporary product or copying its index.
pub fn validate_scene_dependency_kinds(
    snapshot: &SceneSnapshot,
    kind_of: impl Fn(AssetId) -> Option<AssetKind>,
) -> Result<()> {
    for id in snapshot.asset_dependencies() {
        ensure!(kind_of(id).is_some(), "missing dependency {id}");
    }
    validate_entity_dependency_kinds(snapshot, &kind_of)
}

fn expect_kind(
    kind_of: &impl Fn(AssetId) -> Option<AssetKind>,
    id: AssetId,
    expected: AssetKind,
) -> Result<()> {
    let actual = kind_of(id);
    ensure!(
        actual == Some(expected),
        "asset {id} expected {expected:?}, found {actual:?}"
    );
    Ok(())
}

fn validate_entity_dependency_kinds(
    snapshot: &SceneSnapshot,
    kind_of: &impl Fn(AssetId) -> Option<AssetKind>,
) -> Result<()> {
    let expect = |id, kind| expect_kind(kind_of, id, kind);
    for e in &snapshot.entities {
        if let Some(m) = &e.model {
            expect(m.mesh, AssetKind::Mesh)?;
            expect(m.surface.material, AssetKind::Material)?;
        }
        if let Some(script) = &e.script {
            expect(script.asset, AssetKind::Script)?;
        }
        if let Some(animator) = &e.animator {
            expect(animator.skeleton, AssetKind::Skeleton)?;
            if let Some(clip) = animator.clip {
                expect(clip, AssetKind::AnimationClip)?;
            }
        }
        if let Some(skin) = &e.skin {
            expect(skin.skin, AssetKind::Skin)?;
        }
        if let Some(audio) = &e.audio_source {
            expect(audio.clip, AssetKind::Audio)?;
        }
        if let Some(data) = e.navigation_region.as_ref().and_then(|region| region.data) {
            expect(data, AssetKind::Navigation)?;
        }
        for r in schema::entity_references(e) {
            if let EntityRef::External { scene, .. } = r {
                expect(scene, AssetKind::Scene)?;
            }
        }
    }
    Ok(())
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
            "unsupported product version {}; rebuild products from authoring sources",
            self.version
        );
        validate_digest(&self.source_digest)?;
        self.payload.validate()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorProvenance {
    /// Authoring source identity, not a runtime dependency.
    pub source: AssetId,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    pub digest: String,
    pub kind: AssetKind,
    pub dependencies: BTreeSet<AssetId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<EditorProvenance>,
}
impl IndexEntry {
    /// Display metadata does not change the immutable product or its runtime dependencies.
    pub fn same_product(&self, other: &Self) -> bool {
        self.digest == other.digest
            && self.kind == other.kind
            && self.dependencies == other.dependencies
    }
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
            self.version == INDEX_VERSION,
            "unsupported index version {}; rebuild products from authoring sources",
            self.version
        );
        for (id, e) in &self.products {
            validate_digest(&e.digest)?;
            if let Some(editor) = &e.editor {
                ensure!(
                    !editor.label.trim().is_empty()
                        && editor.label.len() <= MAX_EDITOR_LABEL_BYTES
                        && !editor.label.contains('\0'),
                    "asset {id} has an invalid editor label"
                );
                ensure!(
                    self.products
                        .get(&editor.source)
                        .and_then(|source| source.editor.as_ref())
                        .is_some_and(|source| source.source == editor.source),
                    "asset {id} has no editor source root {}",
                    editor.source
                );
            }
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
