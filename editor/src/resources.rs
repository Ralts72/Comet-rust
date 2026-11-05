use crate::jobs::{Event, Job};
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_asset_pipeline::SourceCatalog;
use comet_core::{AssetId, ProjectPaths, WorldId};
use comet_scene::{ContentSource, SceneSnapshot};
use std::{path::Path, sync::Arc};

pub(crate) struct Candidate {
    pub world: WorldId,
    pub document_revision: u64,
    pub assets: AssetStore,
    pub sources: SourceCatalog,
    pub snapshot: SceneSnapshot,
}
#[derive(Default)]
pub(crate) struct ResourceBuild {
    job: Option<Job<Candidate>>,
    pub stage: String,
    pub failure: Option<String>,
}
impl ResourceBuild {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }
    pub fn start(
        &mut self,
        paths: &ProjectPaths,
        products: &Path,
        world: WorldId,
        document_revision: u64,
        retained: Vec<Arc<ContentSource>>,
        prefab_asset: Option<AssetId>,
    ) -> Result<()> {
        ensure!(!self.is_running(), "资源刷新已在进行");
        let paths = paths.clone();
        let products = products.to_owned();
        let job = Job::spawn("comet-resource-build", move |sender| {
            let _ = sender.send(Event::Stage("正在构建项目资源".into()));
            let report =
                comet_asset_pipeline::prepare(paths.root(), &products).context("资源构建失败")?;
            let _ = sender.send(Event::Stage("正在校验当前文档与撤销历史".into()));
            let mut assets = AssetStore::open(&products)?;
            let snapshot = validate_sources(&retained, &mut assets, prefab_asset)?;
            let sources = SourceCatalog::read(&paths, assets.index())?;
            let _ = sender.send(Event::Stage(format!(
                "资源已准备：{} 个更新，{} 个复用",
                report.changed, report.reused
            )));
            Ok(Candidate {
                world,
                document_revision,
                assets,
                sources,
                snapshot,
            })
        })?;
        self.job = Some(job);
        self.stage = "正在刷新资源".into();
        self.failure = None;
        Ok(())
    }
    pub fn poll(&mut self) -> Option<std::result::Result<Candidate, String>> {
        while let Some(job) = &self.job {
            match job.poll()? {
                Event::Stage(stage) => self.stage = stage,
                Event::Finished(result) => {
                    self.job.take();
                    return Some(result);
                }
            }
        }
        None
    }
}

/// Retained undo/redo and savepoint content must remain usable after an asset switch.
pub(crate) fn validate_sources(
    retained: &[Arc<ContentSource>],
    assets: &mut AssetStore,
    prefab_asset: Option<AssetId>,
) -> Result<SceneSnapshot> {
    let kinds = assets
        .index()
        .products
        .iter()
        .map(|(id, entry)| (*id, entry.kind))
        .collect();
    let mut current = None;
    let mut scripts = comet_runtime::ScriptValidator::default();
    for (index, source) in retained.iter().enumerate() {
        let snapshot = match prefab_asset {
            Some(root) => source.expand_prefab(root, assets),
            None => source.expand(assets),
        }
        .context("资源更新与文档或历史中的 Prefab 不兼容")?;
        // Render extraction skips hidden geometry and does not consume external scene links.
        Product::Scene(ContentSource {
            base: snapshot.clone(),
            ..Default::default()
        })
        .validate_dependency_kinds(&kinds)
        .context("资源更新缺少文档或历史依赖的产品，或产品类型不兼容")?;
        validate_authoring(&snapshot, assets).context("资源更新缺少文档或历史依赖的产品")?;
        scripts
            .validate(&snapshot, &assets.script_sources(&snapshot)?)
            .context("资源更新与文档或历史中的脚本不兼容")?;
        if index == 0 {
            current = Some(snapshot);
        }
    }
    current.context("缺少作者文档")
}

/// Incomplete explicit bindings remain editable; every other asset or draw error still rejects.
pub(crate) fn validate_authoring(snapshot: &SceneSnapshot, assets: &mut AssetStore) -> Result<()> {
    allow_unbound(comet_application::extract(snapshot, assets, &[]).map(|_| ()))?;
    let (skeletons, animations) = assets.animation_sources(snapshot)?;
    allow_unbound(comet_runtime::validate_animations(
        snapshot,
        &skeletons,
        &animations,
    ))
}

fn allow_unbound(result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.is::<comet_scene::UnboundRig>() => Ok(()),
        Err(error) => Err(error),
    }
}
