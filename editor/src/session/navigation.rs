use super::{Editor, SnapshotCache, contains_entity};
use crate::navigation::{NavigationGuard, NavigationResult};
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_pipeline::SourceCatalog;
use comet_editor_core::{Document, FieldEdit};
use comet_scene::{
    SceneSnapshot,
    schema::{self, FieldValue},
};
use std::{path::Path, sync::Arc};

impl Editor {
    pub(crate) fn navigation_snapshot(
        &mut self,
        assets: &mut AssetStore,
    ) -> Result<Arc<SceneSnapshot>> {
        ensure!(self.play.is_none(), "运行期间不能读取作者导航输入");
        self.shared_snapshot(assets)
    }

    pub(crate) fn begin_navigation_bake(
        &mut self,
        relative: &Path,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(self.play.is_none(), "请先停止运行，再烘焙导航");
        ensure!(!self.is_busy(), "请等待后台任务完成，再烘焙导航");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        self.verify_document_identity()?;
        self.commit()?;
        let snapshot = self.navigation_snapshot(assets)?;
        let input = self
            .navigation
            .input(&self.document, &snapshot, assets.revision())
            .map_err(anyhow::Error::msg)?;
        let guard = NavigationGuard {
            world: self.document.world_id(),
            document_revision: self.document.revision(),
            index_revision: assets.revision().to_owned(),
            prefab_asset: self.document.prefab_asset(),
            region: input.region(),
            input_digest: input.digest().to_owned(),
        };
        self.picking.clear();
        self.navigation.start(&self.paths, relative, guard, input)
    }

    fn check_navigation_guard(
        &mut self,
        guard: &NavigationGuard,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(
            self.play.is_none() && !self.documents.is_blocking() && !self.document.is_editing(),
            "文档操作使导航候选失效，请重新烘焙"
        );
        ensure!(
            guard.world == self.document.world_id()
                && guard.document_revision == self.document.revision()
                && guard.index_revision == assets.revision()
                && guard.prefab_asset == self.document.prefab_asset(),
            "文档或资源在导航烘焙期间发生变化，请重新烘焙"
        );
        self.verify_document_identity()?;
        let snapshot = self.navigation_snapshot(assets)?;
        let input = self
            .navigation
            .input(&self.document, &snapshot, assets.revision())
            .map_err(anyhow::Error::msg)?;
        ensure!(
            input.region() == guard.region && input.digest() == guard.input_digest,
            "导航几何或参数已变化，请重新烘焙"
        );
        Ok(())
    }

    pub(super) fn poll_navigation(&mut self, assets: &mut AssetStore) {
        let Some(result) = self.navigation.poll() else {
            return;
        };
        let result = result.map_err(anyhow::Error::msg).and_then(|result| {
            match result {
                NavigationResult::Baked(baked) => {
                    self.check_navigation_guard(&baked.guard, assets)?;
                    self.navigation.prepare(
                        &self.paths,
                        &self.products,
                        baked,
                        self.document.retained_sources().collect(),
                    )?;
                }
                NavigationResult::Prepared(mut candidate) => {
                    let output = candidate.output.clone();
                    let accepted = (|| -> Result<()> {
                        self.check_navigation_guard(&candidate.guard, assets)?;
                        let target = self
                            .resolve_field_target(candidate.guard.region, &mut candidate.assets)?;
                        let edits = [FieldEdit {
                            target,
                            field: schema::NAVIGATION_DATA,
                            value: FieldValue::OptionalAsset(Some(candidate.asset)),
                        }];
                        // Validate the complete binding without changing the live document's IDs or history.
                        let source = self.document.source();
                        let mut preview = match self.document.prefab_asset() {
                            Some(root) => {
                                Document::open_prefab(root, source, &mut candidate.assets)?
                            }
                            None => Document::open_source(source)?,
                        };
                        preview.begin()?;
                        preview.preview_field_values(&edits, &mut candidate.assets)?;
                        let snapshot = preview.expand(&mut candidate.assets)?;
                        let navigation = candidate.assets.navigation_data(&snapshot)?;
                        comet_runtime::validate_navigation(&snapshot, navigation.as_deref())?;
                        comet_application::extract(&snapshot, &mut candidate.assets, &[])?;
                        let sources = SourceCatalog::read(&self.paths, candidate.assets.index())?;
                        self.verify_document_identity()?;
                        self.document.begin()?;
                        if let Err(error) = self
                            .document
                            .preview_field_values(&edits, &mut candidate.assets)
                        {
                            self.document.cancel()?;
                            return Err(error);
                        }
                        self.document.commit()?;
                        let snapshot = Arc::new(snapshot);
                        self.selection.retain(|id| contains_entity(&snapshot, id));
                        self.picking.clear();
                        self.source_revision = candidate.assets.revision().to_owned();
                        self.sources = sources;
                        self.export.retain_scenes(&self.sources);
                        self.resource_selection = None;
                        self.snapshot_cache = Some(SnapshotCache {
                            world: self.document.world_id(),
                            document_revision: self.document.revision(),
                            products_revision: self.source_revision.clone(),
                            snapshot: snapshot.clone(),
                        });
                        self.last_snapshot = snapshot;
                        *assets = candidate.assets;
                        self.navigation.accepted();
                        self.status = format!("导航已烘焙：{}", output.display());
                        Ok(())
                    })();
                    accepted.with_context(|| {
                        format!(
                            "导航作者文件已完整保留在 {}，文档和资源未接纳",
                            output.display()
                        )
                    })?;
                }
            }
            Ok(())
        });
        if let Err(error) = result {
            self.navigation.fail(format!("{error:#}"));
        }
    }
}
