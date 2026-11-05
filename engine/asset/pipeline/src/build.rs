use crate::{
    import,
    source::{SourceMeta, scan},
};
use anyhow::{Context, Result, ensure};
use comet_asset_data::product::{
    AssetKind, EditorProvenance, INDEX_VERSION, IndexEntry, PRODUCT_VERSION, Product, ProductFile,
    ProductIndex,
};
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
struct PreparedProduct {
    file: ProductFile,
    label: String,
}
struct Input {
    path: PathBuf,
    meta_path: PathBuf,
    meta: SourceMeta,
    new_meta: bool,
    products: Vec<PreparedProduct>,
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
        let bytes = import::read(path)?;
        let imported = import::bundle(path, &bytes, meta.id)
            .with_context(|| format!("import {}", path.display()))?;
        let mut fingerprint = if imported.files.is_empty() && imported.products.len() == 1 {
            b"comet-assetc-v1\0".to_vec()
        } else {
            b"comet-assetc-gltf-v2\0".to_vec()
        };
        fingerprint.extend_from_slice(&bytes);
        let base = path.parent().context("source directory")?.canonicalize()?;
        for (file, hash) in &imported.files {
            fingerprint.extend_from_slice(file.strip_prefix(&base)?.as_os_str().as_encoded_bytes());
            fingerprint.extend_from_slice(hash.as_bytes());
        }
        let source_digest = digest(&fingerprint);
        let source_label = path
            .strip_prefix(&assets)?
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let mut products = vec![];
        for import::ImportedProduct { id, payload, label } in imported.products {
            if let Some(previous) = origins.insert(id, path.clone()) {
                anyhow::bail!(
                    "duplicate asset ID {id}: {} and {}",
                    previous.display(),
                    path.display()
                );
            }
            payload.validate()?;
            products.push(PreparedProduct {
                file: ProductFile {
                    version: PRODUCT_VERSION,
                    id,
                    source_digest: source_digest.clone(),
                    payload,
                },
                label: if id == meta.id {
                    source_label.clone()
                } else {
                    label
                },
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
        .map(|p| (p.file.id, p.file.payload.clone()))
        .collect();
    let kinds = products.iter().map(|(id, p)| (*id, p.kind())).collect();
    let mut resolver = Sources(&products);
    for input in &inputs {
        for product in &input.products {
            product
                .file
                .payload
                .validate_dependency_kinds(&kinds)
                .with_context(|| format!("dependencies of {}", input.path.display()))?;
            match &product.file.payload {
                Product::Skin(skin) => {
                    let Some(Product::Skeleton(skeleton)) = products.get(&skin.skeleton) else {
                        anyhow::bail!("skin requires skeleton product");
                    };
                    skin.validate_for(skeleton)?;
                }
                Product::AnimationClip(clip) => {
                    let Some(Product::Skeleton(skeleton)) = products.get(&clip.skeleton) else {
                        anyhow::bail!("clip requires skeleton product");
                    };
                    clip.validate_for(skeleton)?;
                }
                _ => {}
            }
            if let Product::Scene(s) | Product::Prefab(s) = &product.file.payload {
                let expanded = s
                    .expand(&mut resolver)
                    .with_context(|| format!("expand {}", input.path.display()))?;
                for entity in &expanded.entities {
                    if let Some(binding) = &entity.skin {
                        let Some(Product::Skin(skin)) = products.get(&binding.skin) else {
                            anyhow::bail!("skin binding requires skin product");
                        };
                        let mesh_id = entity.model.as_ref().context("skin requires model")?.mesh;
                        let Some(Product::Mesh(mesh)) = products.get(&mesh_id) else {
                            anyhow::bail!("skin requires mesh product");
                        };
                        skin.validate_mesh(mesh)?;
                    }
                }
                Product::Scene(ContentSource {
                    base: expanded,
                    ..Default::default()
                })
                .validate_dependency_kinds(&kinds)?;
            }
        }
    }
    let previous = previous_index(output)?;
    let mut index = ProductIndex {
        version: INDEX_VERSION,
        products: BTreeMap::new(),
    };
    for input in &inputs {
        for prepared in &input.products {
            let product = &prepared.file;
            let bytes = serde_json::to_vec(product)?;
            if matches!(product.payload, Product::Audio(_)) {
                ensure!(
                    bytes.len() <= comet_asset_data::audio::MAX_AUDIO_BYTES,
                    "audio product exceeds its encoded size budget"
                );
            }
            if matches!(
                product.payload,
                Product::Mesh(_)
                    | Product::Skeleton(_)
                    | Product::Skin(_)
                    | Product::AnimationClip(_)
            ) {
                ensure!(
                    bytes.len() <= comet_asset_data::MAX_ANIMATION_BYTES,
                    "animation/mesh product exceeds encoded byte budget"
                );
            }
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
                    editor: Some(EditorProvenance {
                        source: input.meta.id,
                        label: prepared.label.clone(),
                    }),
                },
            );
        }
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
        .filter(|(id, e)| {
            previous
                .as_ref()
                .and_then(|p| p.products.get(id))
                .is_none_or(|old| !old.same_product(e))
        })
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
fn previous_index(output: &Path) -> Result<Option<ProductIndex>> {
    let bytes = match fs::read(output.join("index.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read previous product index"),
    };
    let mut previous: ProductIndex = serde_json::from_slice(&bytes)
        .context("parse previous product index; the existing index is invalid")?;
    if matches!(previous.version, 1..=5) {
        let legacy: serde_json::Value = serde_json::from_slice(&bytes)?;
        ensure!(
            previous.version >= 3
                || legacy["products"]
                    .as_object()
                    .context("invalid legacy product map")?
                    .values()
                    .all(|entry| entry.get("editor").is_none()),
            "legacy index contains editor provenance from a newer version"
        );
        ensure!(
            previous.version != 1
                || previous
                    .products
                    .values()
                    .all(|entry| entry.kind != AssetKind::Script),
            "legacy index contains a new asset kind"
        );
        ensure!(
            previous.version >= 4
                || previous
                    .products
                    .values()
                    .all(|entry| entry.kind != AssetKind::Navigation),
            "legacy index contains a newer navigation asset kind"
        );
        ensure!(
            previous.version >= 5
                || previous
                    .products
                    .values()
                    .all(|entry| entry.kind != AssetKind::Audio),
            "legacy index contains a newer audio asset kind"
        );
        ensure!(
            previous.products.values().all(|entry| !matches!(
                entry.kind,
                AssetKind::Skeleton | AssetKind::Skin | AssetKind::AnimationClip
            )),
            "legacy index contains a newer animation asset kind"
        );
        for (id, entry) in &previous.products {
            if entry.kind == AssetKind::Mesh {
                comet_core::validate_digest(&entry.digest)?;
                let path = output
                    .join("products")
                    .join(id.to_string())
                    .join(format!("{}.json", entry.digest));
                let bytes = import::read_bounded(&path, comet_asset_data::MAX_ANIMATION_BYTES)?;
                ensure!(
                    digest(&bytes) == entry.digest,
                    "legacy mesh digest mismatch"
                );
                let file: ProductFile =
                    serde_json::from_slice(&bytes).context("parse legacy mesh product")?;
                let Product::Mesh(mesh) = &file.payload else {
                    anyhow::bail!("legacy mesh kind mismatch");
                };
                ensure!(
                    file.id == *id
                        && mesh.version == 1
                        && mesh.joints.is_none()
                        && mesh.weights.is_none(),
                    "legacy index contains newer mesh skin attributes"
                );
                mesh.validate()?;
            }
        }
        // Only the producer migrates known caches; author identities and current product bytes stay intact.
        previous.version = INDEX_VERSION;
    }
    previous
        .validate()
        .context("validate previous product index")?;
    Ok(Some(previous))
}
fn verify_snapshot(assets: &Path, paths: &[PathBuf], inputs: &[Input]) -> Result<()> {
    let mut current = vec![];
    scan(assets, &mut current)?;
    current.sort();
    ensure!(current == paths, "source set changed during build; retry");
    for i in inputs {
        ensure!(
            digest(&import::read(&i.path)?) == i.source_hash,
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
                digest(&import::read_bounded(
                    file,
                    comet_asset_data::MAX_ANIMATION_BYTES
                )?) == *hash,
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
