//! Authoring identities and the source-to-product catalog belong to the asset producer.
use anyhow::{Context, Result, ensure};
use comet_asset_data::product::{AssetKind, ProductIndex};
use comet_core::{AssetId, ProjectPaths};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMeta {
    pub version: u32,
    pub id: AssetId,
}
impl SourceMeta {
    pub fn path(source: &Path) -> PathBuf {
        let mut path = source.as_os_str().to_owned();
        path.push(".meta");
        path.into()
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let meta: Self = serde_json::from_slice(bytes).context("parse source metadata")?;
        meta.validate()?;
        Ok(meta)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported metadata version {}",
            self.version
        );
        Ok(())
    }
    pub fn load(source: &Path) -> Result<Self> {
        let path = Self::path(source);
        Self::from_bytes(&fs::read(&path).with_context(|| format!("read {}", path.display()))?)
            .with_context(|| format!("metadata {}", path.display()))
    }
    /// Reserve an identity without replacing metadata created since the caller's last check.
    pub(crate) fn reserve(&self, source: &Path) -> Result<()> {
        self.validate()?;
        let path = Self::path(source);
        comet_core::atomic_create(&path, &serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("reserve metadata {}", path.display()))
    }
}

#[derive(Clone, Debug)]
pub struct SourceAsset {
    pub id: AssetId,
    /// Relative to the project's assets directory; paths never define persistent identity.
    pub source: PathBuf,
    pub kind: AssetKind,
}
#[derive(Default)]
pub struct SourceCatalog {
    entries: Vec<SourceAsset>,
}
impl SourceCatalog {
    /// List only built sources, using the same traversal and metadata rules as prepare().
    pub fn read(paths: &ProjectPaths, index: &ProductIndex) -> Result<Self> {
        let mut files = vec![];
        scan(paths.assets(), &mut files)?;
        files.sort();
        let mut entries = vec![];
        let mut ids = std::collections::BTreeSet::new();
        for file in files {
            let file = paths.resolve_asset_path(file)?;
            if !SourceMeta::path(&file).try_exists()? {
                continue;
            }
            let meta = SourceMeta::load(&file)?;
            let Some(product) = index.products.get(&meta.id) else {
                continue;
            };
            ensure!(
                Some(product.kind) == crate::import::source_kind(&file),
                "source/product kind mismatch: {}",
                file.display()
            );
            ensure!(
                product
                    .editor
                    .as_ref()
                    .is_none_or(|origin| origin.source == meta.id),
                "source identity refers to a derived product: {}",
                file.display()
            );
            ensure!(ids.insert(meta.id), "duplicate source identity {}", meta.id);
            entries.push(SourceAsset {
                id: meta.id,
                source: file.strip_prefix(paths.assets())?.to_owned(),
                kind: product.kind,
            });
        }
        Ok(Self { entries })
    }
    pub fn of_kind(&self, kind: AssetKind) -> impl Iterator<Item = &SourceAsset> {
        self.entries.iter().filter(move |entry| entry.kind == kind)
    }
}

pub(crate) fn scan(root: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root).with_context(|| format!("scan {}", root.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "source symlinks are not supported: {}",
            entry.path().display()
        );
        if kind.is_dir() {
            scan(&entry.path(), output)?;
        } else if kind.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            ensure!(
                ![
                    ".scene.json",
                    ".prefab.json",
                    ".mesh.json",
                    ".material.json",
                    ".meta.json"
                ]
                .iter()
                .any(|suffix| name.ends_with(suffix)),
                "legacy asset filename {}; use .scene/.prefab/.mesh/.material and <source-filename>.meta; rename sources and metadata together, preserving metadata IDs",
                entry.path().display()
            );
            if crate::import::source_kind(&entry.path()).is_some() {
                output.push(entry.path());
            }
        }
    }
    Ok(())
}
