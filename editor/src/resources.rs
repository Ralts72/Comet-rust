use crate::jobs::Job;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::ProductIndex;
use comet_asset_pipeline::SourceCatalog;
use comet_core::{AssetId, ProjectPaths, WorldId};
use comet_editor_core::Document;
use comet_scene::SceneSnapshot;
use std::{path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DocumentVersion {
    world: WorldId,
    revision: u64,
    pub prefab_asset: Option<AssetId>,
}
impl DocumentVersion {
    pub fn capture(document: &Document) -> Self {
        Self {
            world: document.world_id(),
            revision: document.revision(),
            prefab_asset: document.prefab_asset(),
        }
    }
    pub fn matches(self, document: &Document) -> bool {
        self.world == document.world_id()
            && self.revision == document.revision()
            && self.prefab_asset == document.prefab_asset()
    }
}
#[derive(Clone, Debug)]
pub(crate) struct ResourceVersion {
    pub document: DocumentVersion,
    index: Arc<ProductIndex>,
}
impl ResourceVersion {
    pub fn capture(document: &Document, assets: &AssetStore) -> Self {
        Self {
            document: DocumentVersion::capture(document),
            index: assets.shared_index(),
        }
    }
    pub fn matches(&self, document: &Document, assets: &AssetStore) -> bool {
        self.document.matches(document) && Arc::ptr_eq(&self.index, &assets.shared_index())
    }
}
pub(crate) struct ResourceCandidate {
    pub document: DocumentVersion,
    pub assets: AssetStore,
    pub sources: SourceCatalog,
    pub snapshot: Arc<SceneSnapshot>,
}
#[derive(Default)]
pub(crate) struct ResourceBuild {
    job: Option<Job<ResourceCandidate>>,
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
        document: &Document,
    ) -> Result<()> {
        ensure!(!self.is_running(), "资源刷新已在进行");
        let version = DocumentVersion::capture(document);
        let retained: Vec<_> = document.retained_sources().collect();
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
                version.prefab_asset,
            )?;
            let sources = SourceCatalog::read(&paths, assets.index())?;
            let _ = sender.send(format!(
                "资源已准备：{} 个更新，{} 个复用",
                report.changed, report.reused
            ));
            Ok(ResourceCandidate {
                document: version,
                assets,
                sources,
                snapshot: Arc::new(snapshot),
            })
        })?;
        self.job = Some(job);
        self.stage = "正在刷新资源".into();
        self.failure = None;
        Ok(())
    }
    pub fn poll(&mut self) -> Option<std::result::Result<ResourceCandidate, String>> {
        let result = self.job.as_mut()?.poll(|stage| self.stage = stage)?;
        self.job.take();
        Some(result)
    }
}
