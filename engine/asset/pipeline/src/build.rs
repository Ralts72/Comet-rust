use crate::{
    import,
    source::{SourceMeta, scan},
};
use anyhow::{Context, Result, ensure};
use comet_asset_data::product::{IndexEntry, PRODUCT_VERSION, Product, ProductFile, ProductIndex};
use comet_core::{AssetId, atomic_write, digest};
use comet_scene::{ContentSource, PrefabResolver};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Serialize)]
pub struct BuildReport {
    pub sources: usize,
    pub products: usize,
    pub changed: usize,
    pub reused: usize,
    pub removed: usize,
    pub index_digest: String,
}
struct Input {
    path: PathBuf,
    meta_path: PathBuf,
    meta: SourceMeta,
    new_meta: bool,
    products: Vec<ProductFile>,
    source_hash: String,
    metadata_hash: Option<String>,
    files: BTreeMap<PathBuf, String>,
}
pub(super) fn lock(path: &Path, description: &str) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock()
        .with_context(|| format!("another asset build holds the {description} lock"))?;
    Ok(file)
}
pub fn prepare(project: &Path, output: &Path) -> Result<BuildReport> {
    let _project_lock = lock(&project.join(".assetc.lock"), "project")?;
    fs::create_dir_all(output)?;
    let _output_lock = lock(&output.join(".build.lock"), "output")?;
    let assets = project.join("assets");
    ensure!(
        assets.is_dir(),
        "missing source directory {}",
        assets.display()
    );
    let mut paths = vec![];
    scan(&assets, &mut paths)?;
    paths.sort();
    ensure!(
        !paths.is_empty(),
        "no supported assets under {}",
        assets.display()
    );
    let mut inputs = vec![];
    let mut origins = BTreeMap::new();
    for path in &paths {
        let meta_path = SourceMeta::path(path);
        let metadata = if meta_path.try_exists()? {
            Some(fs::read(&meta_path)?)
        } else {
            None
        };
        let meta: SourceMeta = match &metadata {
            Some(b) => SourceMeta::from_bytes(b)
                .with_context(|| format!("parse metadata {}", meta_path.display()))?,
            None => SourceMeta {
                version: 1,
                id: AssetId::new(),
            },
        };
        let bytes = fs::read(path)?;
        let imported = import::bundle(path, &bytes, meta.id)
            .with_context(|| format!("import {}", path.display()))?;
        let mut fingerprint = if imported.files.is_empty() && imported.products.len() == 1 {
            b"comet-assetc-v1\0".to_vec()
        } else {
            b"comet-assetc-gltf-v1\0".to_vec()
        };
        fingerprint.extend_from_slice(&bytes);
        let base = path.parent().context("source directory")?.canonicalize()?;
        for (file, hash) in &imported.files {
            fingerprint.extend_from_slice(file.strip_prefix(&base)?.as_os_str().as_encoded_bytes());
            fingerprint.extend_from_slice(hash.as_bytes());
        }
        let source_digest = digest(&fingerprint);
        let mut products = vec![];
        for (id, payload) in imported.products {
            if let Some(previous) = origins.insert(id, path.clone()) {
                anyhow::bail!(
                    "duplicate asset ID {id}: {} and {}",
                    previous.display(),
                    path.display()
                );
            }
            payload.validate()?;
            products.push(ProductFile {
                version: PRODUCT_VERSION,
                id,
                source_digest: source_digest.clone(),
                payload,
            });
        }
        inputs.push(Input {
            path: path.clone(),
            meta_path,
            meta,
            new_meta: metadata.is_none(),
            products,
            source_hash: digest(&bytes),
            metadata_hash: metadata.as_ref().map(|b| digest(b)),
            files: imported.files,
        });
    }
    let products: BTreeMap<_, _> = inputs
        .iter()
        .flat_map(|i| i.products.iter())
        .map(|p| (p.id, p.payload.clone()))
        .collect();
    let kinds = products.iter().map(|(id, p)| (*id, p.kind())).collect();
    let mut resolver = Sources(&products);
    for input in &inputs {
        for product in &input.products {
            product
                .payload
                .validate_dependency_kinds(&kinds)
                .with_context(|| format!("dependencies of {}", input.path.display()))?;
            if let Product::Scene(s) | Product::Prefab(s) = &product.payload {
                let expanded = s
                    .expand(&mut resolver)
                    .with_context(|| format!("expand {}", input.path.display()))?;
                Product::Scene(ContentSource {
                    base: expanded,
                    ..Default::default()
                })
                .validate_dependency_kinds(&kinds)?;
            }
        }
    }
    let previous: Option<ProductIndex> = fs::read(output.join("index.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let mut index = ProductIndex {
        version: 1,
        products: BTreeMap::new(),
    };
    for product in inputs.iter().flat_map(|i| &i.products) {
        let bytes = serde_json::to_vec(product)?;
        let hash = digest(&bytes);
        let path = output
            .join("products")
            .join(product.id.to_string())
            .join(format!("{hash}.json"));
        if !fs::read(&path).is_ok_and(|b| b == bytes) {
            atomic_write(&path, &bytes)?;
        }
        index.products.insert(
            product.id,
            IndexEntry {
                digest: hash,
                kind: product.payload.kind(),
                dependencies: product.payload.dependencies(),
            },
        );
    }
    index.validate()?;
    verify_snapshot(&assets, &paths, &inputs)?;
    for input in &inputs {
        if input.new_meta {
            atomic_write(&input.meta_path, &serde_json::to_vec_pretty(&input.meta)?)?;
        }
    }
    let bytes = serde_json::to_vec_pretty(&index)?;
    let changed = index
        .products
        .iter()
        .filter(|(id, e)| previous.as_ref().and_then(|p| p.products.get(id)) != Some(e))
        .count();
    let removed = previous.as_ref().map_or(0, |p| {
        p.products
            .keys()
            .filter(|id| !index.products.contains_key(id))
            .count()
    });
    atomic_write(&output.join("index.json"), &bytes)?;
    Ok(BuildReport {
        sources: inputs.len(),
        products: index.products.len(),
        changed,
        reused: index.products.len() - changed,
        removed,
        index_digest: digest(&bytes),
    })
}
fn verify_snapshot(assets: &Path, paths: &[PathBuf], inputs: &[Input]) -> Result<()> {
    let mut current = vec![];
    scan(assets, &mut current)?;
    current.sort();
    ensure!(current == paths, "source set changed during build; retry");
    for i in inputs {
        ensure!(
            digest(&fs::read(&i.path)?) == i.source_hash,
            "source changed during build: {}; retry",
            i.path.display()
        );
        let current = if i.meta_path.try_exists()? {
            Some(digest(&fs::read(&i.meta_path)?))
        } else {
            None
        };
        ensure!(
            current == i.metadata_hash,
            "metadata changed during build: {}; retry",
            i.meta_path.display()
        );
        for (file, hash) in &i.files {
            ensure!(
                digest(&fs::read(file)?) == *hash,
                "glTF resource changed during build: {}; retry",
                file.display()
            );
        }
    }
    Ok(())
}
struct Sources<'a>(&'a BTreeMap<AssetId, Product>);
impl PrefabResolver for Sources<'_> {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        match self.0.get(&id) {
            Some(Product::Prefab(s)) => Ok(s.clone()),
            _ => anyhow::bail!("missing prefab product {id}"),
        }
    }
}
