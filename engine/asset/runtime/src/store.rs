use anyhow::{Context, Result, ensure};
use comet_asset_data::product::{
    Product, ProductFile, ProductIndex, validate_scene_dependency_kinds,
};
use comet_core::{AssetId, digest};
use comet_scene::{ContentSource, PrefabResolver, SceneSnapshot};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct AssetStore {
    root: PathBuf,
    revision: String,
    index: Arc<ProductIndex>,
    cache: BTreeMap<AssetId, Arc<ProductFile>>,
}
impl AssetStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_owned();
        let bytes = fs::read(root.join("index.json")).context("read product index")?;
        let revision = digest(&bytes);
        let index: ProductIndex = serde_json::from_slice(&bytes).context("parse product index")?;
        index.validate()?;
        let mut store = Self {
            root,
            revision,
            index: Arc::new(index),
            cache: BTreeMap::new(),
        };
        let ids: Vec<_> = store.index.products.keys().copied().collect();
        for id in &ids {
            store.load(*id)?;
        }
        // Check expansion without an ECS world, including nested overrides and prefab cycles.
        for id in ids {
            let p = store.load(id)?;
            if let Product::Scene(s) | Product::Prefab(s) = &p.payload {
                let expanded = s
                    .expand(&mut store)
                    .with_context(|| format!("expand product {id}"))?;
                validate_scene_dependency_kinds(&expanded, |id| {
                    store.index.products.get(&id).map(|entry| entry.kind)
                })?;
            }
        }
        store.cache.clear();
        Ok(store)
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn index(&self) -> &ProductIndex {
        &self.index
    }
    /// Immutable identity of this Store's accepted index; unchanged refreshes retain it.
    pub fn shared_index(&self) -> Arc<ProductIndex> {
        self.index.clone()
    }
    pub fn load(&mut self, id: AssetId) -> Result<Arc<ProductFile>> {
        if let Some(p) = self.cache.get(&id) {
            return Ok(p.clone());
        }
        let entry = self
            .index
            .products
            .get(&id)
            .with_context(|| format!("asset not indexed {id}"))?;
        let path = self
            .root
            .join("products")
            .join(id.to_string())
            .join(format!("{}.json", entry.digest));
        let bytes = if let Some(limit) = entry.kind.encoded_byte_limit() {
            let file =
                fs::File::open(&path).with_context(|| format!("read bounded product {id}"))?;
            comet_core::read_bounded(file, limit)
                .with_context(|| format!("read product {id} within its file size budget"))?
        } else {
            fs::read(&path).with_context(|| format!("read product {}", path.display()))?
        };
        ensure!(
            digest(&bytes) == entry.digest,
            "product digest mismatch {id}"
        );
        let file: ProductFile =
            serde_json::from_slice(&bytes).with_context(|| format!("parse product {id}"))?;
        file.validate()
            .with_context(|| format!("validate product {id}"))?;
        ensure!(
            file.id == id
                && file.payload.kind() == entry.kind
                && file.payload.dependencies() == entry.dependencies,
            "product/index mismatch {id}"
        );
        file.payload
            .validate_dependency_kinds(|id| self.index.products.get(&id).map(|entry| entry.kind))?;
        match &file.payload {
            Product::Skin(skin) => {
                let skeleton = self.load(skin.skeleton)?;
                let Product::Skeleton(skeleton) = &skeleton.payload else {
                    anyhow::bail!("skin requires a skeleton");
                };
                skin.validate_for(skeleton)?;
            }
            Product::AnimationClip(clip) => {
                let skeleton = self.load(clip.skeleton)?;
                let Product::Skeleton(skeleton) = &skeleton.payload else {
                    anyhow::bail!("clip requires a skeleton");
                };
                clip.validate_for(skeleton)?;
            }
            Product::AnimationController(controller) => {
                for clip in controller.clips() {
                    let clip = self.load(clip)?;
                    let Product::AnimationClip(clip) = &clip.payload else {
                        anyhow::bail!("controller requires animation clips");
                    };
                    controller.validate_clip(clip)?;
                }
            }
            _ => {}
        }
        let file = Arc::new(file);
        self.cache.insert(id, file.clone());
        Ok(file)
    }
    /// Accept a complete index and return content changes; editor metadata keeps product Arcs reusable.
    pub fn refresh(&mut self) -> Result<BTreeSet<AssetId>> {
        let mut candidate = Self::open(&self.root)?;
        let ids: BTreeSet<_> = self
            .index
            .products
            .keys()
            .chain(candidate.index.products.keys())
            .copied()
            .collect();
        let changed = ids
            .into_iter()
            .filter(|id| {
                self.index
                    .products
                    .get(id)
                    .zip(candidate.index.products.get(id))
                    .is_none_or(|(old, new)| !old.same_product(new))
            })
            .collect();
        for (id, p) in &self.cache {
            if self
                .index
                .products
                .get(id)
                .zip(candidate.index.products.get(id))
                .is_some_and(|(old, new)| old.same_product(new))
            {
                candidate.cache.insert(*id, p.clone());
            }
        }
        if candidate.revision == self.revision {
            candidate.index = self.index.clone();
        }
        *self = candidate;
        Ok(changed)
    }
    pub fn dependency_closure(&self, root: AssetId) -> Result<BTreeSet<AssetId>> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if seen.insert(id) {
                pending.extend(
                    self.index
                        .products
                        .get(&id)
                        .with_context(|| format!("missing asset {id}"))?
                        .dependencies
                        .iter()
                        .copied(),
                );
            }
        }
        Ok(seen)
    }
    pub fn expand_scene(&mut self, id: AssetId) -> Result<SceneSnapshot> {
        let p = self.load(id)?;
        match &p.payload {
            Product::Scene(s) => s.expand(self),
            _ => anyhow::bail!("asset {id} is not a scene"),
        }
    }
    pub fn evict(&mut self, id: AssetId) {
        self.cache.remove(&id);
    }
    pub fn resident_count(&self) -> usize {
        self.cache.len()
    }
}
impl PrefabResolver for AssetStore {
    fn prefab(&mut self, id: AssetId) -> Result<ContentSource> {
        let p = self.load(id)?;
        match &p.payload {
            Product::Prefab(s) => Ok(s.clone()),
            _ => anyhow::bail!("asset {id} is not a prefab"),
        }
    }
}
