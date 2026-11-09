use crate::jobs::Job;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
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
            let _ = sender.send("正在构建项目资源".into());
            let report =
                comet_asset_pipeline::prepare(paths.root(), &products).context("资源构建失败")?;
            let _ = sender.send("正在校验当前文档与撤销历史".into());
            let mut assets = AssetStore::open(&products)?;
            let snapshot = crate::validation::validate_sources(
                retained.iter().map(|source| source.as_ref()),
                &mut assets,
                prefab_asset,
            )?;
            let sources = SourceCatalog::read(&paths, assets.index())?;
            let _ = sender.send(format!(
                "资源已准备：{} 个更新，{} 个复用",
                report.changed, report.reused
            ));
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
        let result = self.job.as_mut()?.poll(|stage| self.stage = stage)?;
        self.job.take();
        Some(result)
    }
}
