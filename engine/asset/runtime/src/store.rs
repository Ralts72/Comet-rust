use anyhow::{Context, Result, ensure};
use comet_asset_data::product::{Product, ProductFile, ProductIndex};
use comet_core::{AssetId, digest};
use comet_scene::{ContentSource, PrefabResolver, SceneSnapshot};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
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
                let kinds = store
                    .index
                    .products
                    .iter()
                    .map(|(id, e)| (*id, e.kind))
                    .collect();
                Product::Scene(ContentSource {
                    base: expanded,
                    ..Default::default()
                })
                .validate_dependency_kinds(&kinds)?;
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
        let bytes = if matches!(
            entry.kind,
            comet_asset_data::product::AssetKind::Audio
                | comet_asset_data::product::AssetKind::Mesh
                | comet_asset_data::product::AssetKind::Skeleton
                | comet_asset_data::product::AssetKind::Skin
                | comet_asset_data::product::AssetKind::AnimationClip
        ) {
            let file =
                fs::File::open(&path).with_context(|| format!("read bounded product {id}"))?;
            let limit = if entry.kind == comet_asset_data::product::AssetKind::Audio {
                comet_asset_data::audio::MAX_AUDIO_BYTES
            } else {
                comet_asset_data::MAX_ANIMATION_BYTES
            } as u64;
            ensure!(
                file.metadata()?.len() <= limit,
                "product exceeds its file size budget: {id}"
            );
            let mut bytes = Vec::new();
            file.take(limit + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= limit,
                "product grew beyond its file size budget: {id}"
            );
            bytes
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
        let kinds = self
            .index
            .products
            .iter()
            .map(|(id, e)| (*id, e.kind))
            .collect();
        file.payload.validate_dependency_kinds(&kinds)?;
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
    /// Load the explicit CPU navigation input; the simulation owns its live state and compatibility checks.
    pub fn navigation_data(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<Option<Arc<comet_asset_data::NavigationData>>> {
        snapshot.validate()?;
        let Some(region) = snapshot
            .entities
            .iter()
            .find_map(|entity| entity.navigation_region.as_ref())
        else {
            return Ok(None);
        };
        let id = region.data.context("导航区域尚未烘焙，请先生成导航数据")?;
        let product = self.load(id)?;
        let Product::Navigation(data) = &product.payload else {
            anyhow::bail!("导航区域引用的产品不是导航数据：{id}");
        };
        Ok(Some(Arc::new(data.clone())))
    }
    /// Preload only this world's clips; the simulation and output retain immutable PCM independently.
    pub fn audio_sources(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<BTreeMap<AssetId, Arc<comet_asset_data::AudioData>>> {
        snapshot.validate()?;
        let ids: BTreeSet<_> = snapshot
            .entities
            .iter()
            .filter_map(|entity| entity.audio_source.as_ref().map(|source| source.clip))
            .collect();
        let mut sources = BTreeMap::new();
        for id in ids {
            let product = self.load(id)?;
            let Product::Audio(data) = &product.payload else {
                anyhow::bail!("声音源引用的产品不是音频：{id}");
            };
            sources.insert(id, Arc::new(data.clone()));
        }
        Ok(sources)
    }
    /// Resource preparation accepts explicit authoring gaps; runtime validates live bindings.
    pub fn animation_sources(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<(
        comet_asset_data::SkeletonLibrary,
        comet_asset_data::AnimationLibrary,
    )> {
        snapshot.validate()?;
        let mut skeleton_ids = BTreeSet::new();
        let mut clip_ids = BTreeSet::new();
        for entity in &snapshot.entities {
            if let Some(animator) = &entity.animator {
                skeleton_ids.insert(animator.skeleton);
                clip_ids.extend(animator.clip);
            }
            if let Some(binding) = &entity.skin {
                let product = self.load(binding.skin)?;
                let Product::Skin(skin) = &product.payload else {
                    anyhow::bail!("skin binding requires a skin product");
                };
                skeleton_ids.insert(skin.skeleton);
                let mesh = self.load(entity.model.as_ref().context("skin requires model")?.mesh)?;
                let Product::Mesh(mesh) = &mesh.payload else {
                    anyhow::bail!("skin requires a mesh product");
                };
                skin.validate_mesh(mesh)?;
            }
        }
        let mut animations = comet_asset_data::AnimationLibrary::new();
        for id in clip_ids {
            let product = self.load(id)?;
            let Product::AnimationClip(clip) = &product.payload else {
                anyhow::bail!("animator clip requires an animation clip product");
            };
            skeleton_ids.insert(clip.skeleton);
            animations.insert(id, Arc::new(clip.clone()));
        }
        let mut skeletons = comet_asset_data::SkeletonLibrary::new();
        for id in skeleton_ids {
            let product = self.load(id)?;
            let Product::Skeleton(skeleton) = &product.payload else {
                anyhow::bail!("animator requires a skeleton product");
            };
            skeletons.insert(id, Arc::new(skeleton.clone()));
        }
        Ok((skeletons, animations))
    }
    /// Return enabled bindings' source text; Runtime compiles it without owning an AssetStore.
    pub fn script_sources(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<BTreeMap<AssetId, String>> {
        snapshot.validate()?;
        let kinds = self
            .index
            .products
            .iter()
            .map(|(id, entry)| (*id, entry.kind))
            .collect();
        Product::Scene(ContentSource {
            base: snapshot.clone(),
            ..Default::default()
        })
        .validate_dependency_kinds(&kinds)?;
        let ids: BTreeSet<_> = snapshot
            .entities
            .iter()
            .filter_map(|entity| entity.script.as_ref())
            .filter(|script| script.enabled)
            .map(|script| script.asset)
            .collect();
        ids.into_iter()
            .map(|id| {
                let product = self.load(id)?;
                let Product::Script(script) = &product.payload else {
                    anyhow::bail!("asset {id} is not a script")
                };
                Ok((id, script.source.clone()))
            })
            .collect()
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
