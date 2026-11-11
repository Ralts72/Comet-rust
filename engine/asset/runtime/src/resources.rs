//! Scene resource loading stays independent of simulation and editor authoring policy.
use crate::AssetStore;
use anyhow::{Context, Result};
use comet_asset_data::{
    AudioLibrary, MAX_SCENE_DATA_ASSETS, SceneResources, ScriptLibrary,
    product::{Product, validate_scene_dependency_kinds},
};
use comet_scene::SceneSnapshot;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

impl AssetStore {
    /// Prepare a complete candidate under one snapshot/dependency check, without running code.
    pub fn scene_resources(&mut self, snapshot: &SceneSnapshot) -> Result<SceneResources> {
        self.validate_resource_dependencies(snapshot)?;
        let scripts = self
            .load_script_sources(snapshot)
            .context("load scene scripts")?;
        let data = self
            .load_game_data_sources(snapshot)
            .context("load scene data")?;
        let navigation = self
            .load_navigation_data(snapshot)
            .context("load scene navigation")?;
        let audio = self
            .load_audio_sources(snapshot)
            .context("load scene audio")?;
        let animation = self
            .load_animation_sources(snapshot)
            .context("load scene animation")?;
        Ok(SceneResources {
            data,
            scripts,
            navigation,
            audio,
            skeletons: animation.skeletons,
            animations: animation.clips,
            controllers: animation.controllers,
        })
    }

    /// Authoring consumers can prepare a subset while explicit bindings remain incomplete.
    pub fn animation_sources(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<comet_asset_data::AnimationLibraries> {
        snapshot.validate()?;
        self.load_animation_sources(snapshot)
    }
    pub fn navigation_data(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<Option<Arc<comet_asset_data::NavigationData>>> {
        snapshot.validate()?;
        self.load_navigation_data(snapshot)
    }

    /// Script preparation also checks references on disabled scripts and hidden geometry.
    pub fn script_sources(&mut self, snapshot: &SceneSnapshot) -> Result<ScriptLibrary> {
        self.validate_resource_dependencies(snapshot)?;
        self.load_script_sources(snapshot)
    }

    fn validate_resource_dependencies(&self, snapshot: &SceneSnapshot) -> Result<()> {
        snapshot.validate()?;
        validate_scene_dependency_kinds(snapshot, |id| {
            self.index().products.get(&id).map(|entry| entry.kind)
        })
    }
    fn load_navigation_data(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<Option<Arc<comet_asset_data::NavigationData>>> {
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
    fn load_game_data_sources(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<comet_asset_data::game_data::DataLibrary> {
        let mut pending: Vec<_> = snapshot
            .entities
            .iter()
            .filter_map(|e| e.script.as_ref())
            .filter(|s| s.enabled)
            .flat_map(|s| comet_scene::schema::script_data_assets(&s.parameters))
            .collect();
        let mut data = comet_asset_data::game_data::DataLibrary::new();
        while let Some(id) = pending.pop() {
            if data.contains_key(&id) {
                continue;
            }
            anyhow::ensure!(
                data.len() < MAX_SCENE_DATA_ASSETS,
                "runtime data asset budget exceeded"
            );
            let product = self.load(id)?;
            let Product::Data(table) = &product.payload else {
                anyhow::bail!("script data parameter is not data: {id}")
            };
            pending.extend(table.references().filter_map(|(id, kind)| {
                (kind == comet_asset_data::product::AssetKind::Data).then_some(id)
            }));
            data.insert(id, Arc::new(table.clone()));
        }
        Ok(data)
    }
    fn load_audio_sources(&mut self, snapshot: &SceneSnapshot) -> Result<AudioLibrary> {
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
    fn load_animation_sources(
        &mut self,
        snapshot: &SceneSnapshot,
    ) -> Result<comet_asset_data::AnimationLibraries> {
        let mut skeleton_ids = BTreeSet::new();
        let mut clip_ids = BTreeSet::new();
        let mut controllers = comet_asset_data::ControllerLibrary::new();
        for entity in &snapshot.entities {
            if let Some(animator) = &entity.animator {
                skeleton_ids.insert(animator.skeleton);
                clip_ids.extend(animator.clip);
                if let Some(id) = animator.controller {
                    let product = self.load(id)?;
                    let Product::AnimationController(controller) = &product.payload else {
                        anyhow::bail!(
                            "animator controller requires an animation controller product"
                        );
                    };
                    skeleton_ids.insert(controller.skeleton);
                    clip_ids.extend(controller.clips());
                    controllers.insert(id, Arc::new(controller.clone()));
                }
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
        Ok(comet_asset_data::AnimationLibraries {
            skeletons,
            clips: animations,
            controllers,
        })
    }
    fn load_script_sources(&mut self, snapshot: &SceneSnapshot) -> Result<ScriptLibrary> {
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
}
