use super::{Editor, contains_entity};
use crate::resources::{ResourceCandidate, ResourceVersion};
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_pipeline::{PackageOutput, SourceCatalog};
use comet_editor_core::Document;
use comet_scene::SceneSnapshot;
use std::{sync::Arc, time::Duration};

pub(super) struct SnapshotCache {
    version: ResourceVersion,
    snapshot: Arc<SceneSnapshot>,
}
impl SnapshotCache {
    pub(super) fn new(
        document: &Document,
        assets: &AssetStore,
        snapshot: Arc<SceneSnapshot>,
    ) -> Self {
        Self {
            version: ResourceVersion::capture(document, assets),
            snapshot,
        }
    }
}

impl Editor {
    pub(crate) fn refresh_resources(&mut self) -> Result<()> {
        ensure!(!self.is_busy(), "请等待当前后台任务完成");
        ensure!(self.play.is_none(), "请先停止运行，再刷新资源");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        self.verify_document_identity()?;
        self.commit()?;
        self.resource_build
            .start(&self.paths, &self.products, &self.document)
    }

    pub(super) fn poll_resources(&mut self, assets: &mut AssetStore) {
        let Some(result) = self.resource_build.poll() else {
            return;
        };
        match result
            .map_err(anyhow::Error::msg)
            .and_then(|candidate| self.accept_resources(candidate, assets))
        {
            Ok(()) => self.status = "资源刷新完成".into(),
            Err(error) => {
                self.resource_build.stage = "资源刷新失败".into();
                self.resource_build.failure = Some(format!("{error:#}"));
            }
        }
    }

    fn accept_resources(
        &mut self,
        candidate: ResourceCandidate,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(
            self.play.is_none() && !self.document.is_editing(),
            "文档操作使资源候选失效，请重试"
        );
        ensure!(
            candidate.document.matches(&self.document),
            "文档在资源准备期间发生变化，请重试"
        );
        self.verify_document_identity().context("作者身份变化")?;
        self.publish_resources(
            candidate.assets,
            candidate.sources,
            candidate.snapshot,
            assets,
        );
        Ok(())
    }

    // All fallible validation and document changes precede this single publication boundary.
    pub(super) fn publish_resources(
        &mut self,
        candidate: AssetStore,
        sources: SourceCatalog,
        snapshot: Arc<SceneSnapshot>,
        assets: &mut AssetStore,
    ) {
        self.selection.retain(|id| contains_entity(&snapshot, id));
        self.picking.clear();
        self.export.retain_scenes(&sources);
        self.sources = sources;
        self.resource_selection = None;
        self.snapshot_cache = Some(SnapshotCache::new(
            &self.document,
            &candidate,
            snapshot.clone(),
        ));
        self.last_snapshot = snapshot;
        *assets = candidate;
    }

    pub(super) fn shared_snapshot(
        &mut self,
        assets: &mut AssetStore,
    ) -> Result<Arc<SceneSnapshot>> {
        if let Some(play) = &mut self.play {
            return Ok(play.shared_snapshot());
        }
        if let Some(cache) = &self.snapshot_cache
            && cache.version.matches(&self.document, assets)
        {
            return Ok(cache.snapshot.clone());
        }
        let snapshot = Arc::new(self.document.expand(assets)?);
        self.snapshot_cache = Some(SnapshotCache::new(&self.document, assets, snapshot.clone()));
        Ok(snapshot)
    }

    pub(crate) fn poll_export(&mut self, assets: &mut AssetStore) {
        let Some(completed) = self.export.poll() else {
            return;
        };
        let accepted = (|| {
            ensure!(
                completed.paths.root() == self.paths.root() && completed.source == self.source,
                "导出来源文档已切换，当前资源未接纳"
            );
            self.accept_resources(completed.resources, assets)
        })();
        self.export
            .finish(completed.output, completed.run, accepted);
    }

    pub(crate) fn wait_export(&mut self, assets: &mut AssetStore) -> Result<PackageOutput> {
        while self.export.is_running() {
            self.poll_export(assets);
            std::thread::sleep(Duration::from_millis(10));
        }
        self.export.completed_output()
    }
}
