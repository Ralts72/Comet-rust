use crate::{jobs::Job, session::Editor};
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::{
    navigation::{NavigationData, NavigationInput},
    product::Product,
};
use comet_asset_pipeline::NavigationTarget;
use comet_core::{AssetId, EntityId, ProjectPaths, WorldId};
use comet_editor_core::{Document, FieldEdit};
use comet_scene::{
    ContentSource, EntityData, NavigationAgent, NavigationRegion, SceneSnapshot,
    schema::{self, FieldKey, FieldValue},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) const REGION_FIELDS: &[FieldKey] = &[
    schema::NAVIGATION_DATA,
    schema::NAVIGATION_HALF_EXTENTS,
    schema::NAVIGATION_CELL_SIZE,
    schema::NAVIGATION_AGENT_RADIUS,
    schema::NAVIGATION_AGENT_HEIGHT,
    schema::NAVIGATION_MAX_CLIMB,
    schema::NAVIGATION_MAX_SLOPE,
];
pub(crate) const AGENT_FIELDS: &[FieldKey] = &[
    schema::NAVIGATION_ENABLED,
    schema::NAVIGATION_TARGET,
    schema::NAVIGATION_ARRIVAL_DISTANCE,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NavigationGuard {
    pub world: WorldId,
    pub document_revision: u64,
    pub index_revision: String,
    pub prefab_asset: Option<AssetId>,
    pub region: EntityId,
    pub input_digest: String,
}

pub(crate) struct BakedNavigation {
    pub guard: NavigationGuard,
    target: NavigationTarget,
    data: NavigationData,
}
pub(crate) struct NavigationCandidate {
    pub guard: NavigationGuard,
    pub asset: AssetId,
    pub output: PathBuf,
    pub assets: AssetStore,
}
pub(crate) enum NavigationResult {
    Baked(BakedNavigation),
    Prepared(NavigationCandidate),
}

#[derive(PartialEq, Eq)]
struct CacheKey {
    world: WorldId,
    revision: u64,
    index: String,
    prefab: Option<AssetId>,
}
struct CachedNavigation {
    key: CacheKey,
    region: Option<EntityId>,
    input: Option<std::result::Result<Arc<NavigationInput>, String>>,
    status: Option<std::result::Result<(), String>>,
}

#[derive(Default)]
pub(crate) struct NavigationController {
    job: Option<Job<NavigationResult>>,
    cache: Option<CachedNavigation>,
    pub stage: String,
    pub failure: Option<String>,
}

impl NavigationController {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn cached(
        &mut self,
        document: &Document,
        snapshot: &SceneSnapshot,
        index: &str,
    ) -> &mut CachedNavigation {
        let matches = self.cache.as_ref().is_some_and(|cache| {
            cache.key.world == document.world_id()
                && cache.key.revision == document.revision()
                && cache.key.index == index
                && cache.key.prefab == document.prefab_asset()
        });
        if !matches {
            self.cache = Some(CachedNavigation {
                key: CacheKey {
                    world: document.world_id(),
                    revision: document.revision(),
                    index: index.to_owned(),
                    prefab: document.prefab_asset(),
                },
                region: snapshot
                    .entities
                    .iter()
                    .find(|entity| entity.navigation_region.is_some())
                    .map(|entity| entity.id),
                input: None,
                status: None,
            });
        }
        self.cache.as_mut().unwrap()
    }

    pub fn region(
        &mut self,
        document: &Document,
        snapshot: &SceneSnapshot,
        index: &str,
    ) -> Option<EntityId> {
        self.cached(document, snapshot, index).region
    }

    pub fn input(
        &mut self,
        document: &Document,
        snapshot: &SceneSnapshot,
        index: &str,
    ) -> std::result::Result<Arc<NavigationInput>, String> {
        self.cached(document, snapshot, index)
            .input
            .get_or_insert_with(|| {
                NavigationInput::capture(snapshot)
                    .map(Arc::new)
                    .map_err(|error| format!("{error:#}"))
            })
            .clone()
    }

    pub fn status(
        &mut self,
        document: &Document,
        snapshot: &SceneSnapshot,
        assets: &mut AssetStore,
    ) -> std::result::Result<(), String> {
        let input = self.input(document, snapshot, assets.revision());
        let cache = self.cached(document, snapshot, assets.revision());
        cache
            .status
            .get_or_insert_with(|| {
                let input = input?;
                let region = snapshot
                    .entities
                    .iter()
                    .find(|entity| entity.id == input.region())
                    .and_then(|entity| entity.navigation_region.as_ref())
                    .ok_or("导航区域已不存在")?;
                let asset = region.data.ok_or("尚未烘焙导航数据")?;
                let product = assets
                    .load(asset)
                    .map_err(|error| format!("导航数据不可用：{error:#}"))?;
                let Product::Navigation(data) = &product.payload else {
                    return Err("导航区域需要导航数据资产".into());
                };
                data.validate_input(&input)
                    .map_err(|error| format!("导航数据已陈旧或与输入不兼容，请重新烘焙：{error:#}"))
            })
            .clone()
    }

    pub fn start(
        &mut self,
        paths: &ProjectPaths,
        relative: &Path,
        guard: NavigationGuard,
        input: Arc<NavigationInput>,
    ) -> Result<()> {
        ensure!(!self.is_running(), "导航烘焙已在进行");
        ensure!(
            input.region() == guard.region && input.digest() == guard.input_digest,
            "导航输入与烘焙请求不匹配"
        );
        let target = NavigationTarget::open(paths, relative)?;
        self.job = Some(Job::spawn("comet-navigation-bake", move |sender| {
            let _ = sender.send("正在烘焙导航多边形".into());
            let data = comet_asset_pipeline::bake_navigation(&input)?;
            Ok(NavigationResult::Baked(BakedNavigation {
                guard,
                target,
                data,
            }))
        })?);
        self.stage = "正在准备导航烘焙".into();
        self.failure = None;
        Ok(())
    }

    pub fn prepare(
        &mut self,
        paths: &ProjectPaths,
        products: &Path,
        baked: BakedNavigation,
        retained: Vec<Arc<ContentSource>>,
    ) -> Result<()> {
        ensure!(!self.is_running(), "导航烘焙已在进行");
        let paths = paths.clone();
        let products = products.to_owned();
        self.job = Some(Job::spawn("comet-navigation-publish", move |sender| {
            let output = baked.target.path().to_owned();
            let _ = sender.send("正在发布导航作者文件".into());
            let asset =
                comet_asset_pipeline::publish_navigation(&paths, &baked.target, &baked.data)?;
            let prepared = (|| -> Result<AssetStore> {
                let _ = sender.send("正在构建导航资源并校验文档历史".into());
                comet_asset_pipeline::prepare(paths.root(), &products)?;
                let mut assets = AssetStore::open(&products)?;
                crate::validation::validate_sources(
                    retained.iter().map(|source| source.as_ref()),
                    &mut assets,
                    baked.guard.prefab_asset,
                )?;
                Ok(assets)
            })();
            let assets = prepared.with_context(|| {
                format!(
                    "导航作者文件已完整保存并保留在 {}，资源接纳未完成",
                    output.display()
                )
            })?;
            Ok(NavigationResult::Prepared(NavigationCandidate {
                guard: baked.guard,
                asset,
                output,
                assets,
            }))
        })?);
        self.stage = "正在准备导航资源发布".into();
        Ok(())
    }

    pub fn poll(&mut self) -> Option<std::result::Result<NavigationResult, String>> {
        let result = self.job.as_mut()?.poll(|stage| self.stage = stage)?;
        self.job.take();
        if let Err(error) = &result {
            self.fail(error.clone());
        }
        Some(result)
    }
    pub fn fail(&mut self, error: String) {
        self.stage = "导航烘焙失败".into();
        self.failure = Some(error);
    }
    pub fn accepted(&mut self) {
        self.cache = None;
        self.stage = "导航烘焙完成".into();
        self.failure = None;
    }
}

impl Editor {
    pub(crate) fn edit_navigation(
        &mut self,
        id: EntityId,
        region: Option<NavigationRegion>,
        mut agent: Option<NavigationAgent>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        if let Some(agent) = &mut agent {
            let target = self.resolve_field_target(id, assets)?;
            let mut value = FieldValue::Entity(agent.target.take());
            self.source_field_value(target, &mut value, assets)?;
            let FieldValue::Entity(reference) = value else {
                unreachable!()
            };
            agent.target = reference;
        }
        if let Some(asset) = region.as_ref().and_then(|region| region.data) {
            ensure!(
                matches!(&assets.load(asset)?.payload, Product::Navigation(_)),
                "导航区域必须引用导航数据资产"
            );
        }
        self.preview_document(assets, |document, assets| {
            document.preview_navigation(id, region, agent, assets)
        })
        .context("无法更新导航组件，请检查区域、角色与目标引用")
    }

    pub(crate) fn edit_navigation_fields(
        &mut self,
        before: &EntityData,
        after: &EntityData,
        assets: &mut AssetStore,
    ) -> Result<()> {
        self.ensure_authoring_editable()?;
        let target = self.resolve_field_target(before.id, assets)?;
        let mut edits = vec![];
        for &key in REGION_FIELDS.iter().chain(AGENT_FIELDS) {
            let field = schema::descriptor(key)?;
            let original = field.read(before);
            let next = field.read(after);
            if original == next {
                continue;
            }
            ensure!(original.is_some(), "请在源 Prefab 中添加导航组件");
            let mut value = next.context("请在源 Prefab 中移除导航组件")?;
            self.source_field_value(target, &mut value, assets)?;
            if let FieldValue::OptionalAsset(Some(asset)) = &value {
                ensure!(
                    matches!(&assets.load(*asset)?.payload, Product::Navigation(_)),
                    "导航区域必须引用导航数据资产"
                );
            }
            edits.push(FieldEdit {
                target,
                field: key,
                value,
            });
        }
        if edits.is_empty() {
            return Ok(());
        }
        self.preview_document(assets, |document, assets| {
            document.preview_field_values(&edits, assets)
        })
    }
}

#[cfg(test)]
mod tests;
