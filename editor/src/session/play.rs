use super::*;

impl Editor {
    pub fn start(&mut self, assets: &mut AssetStore) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再运行场景");
        self.commit()?;
        let snapshot = self.document.expand(assets)?;
        let scope = if self.document.prefab_asset().is_none() {
            let path = self.paths.resolve_asset_path(&self.source)?;
            let entry = self
                .sources
                .of_kind(comet_asset_data::product::AssetKind::Scene)
                .find(|entry| self.paths.assets().join(&entry.source) == path)
                .context("场景身份未准备，请刷新资源后重试")?;
            ensure!(
                comet_asset_pipeline::SourceMeta::load(&path)?.id == entry.id,
                "场景身份已改变，请刷新资源后重试"
            );
            Some(entry.id)
        } else {
            None
        };
        let candidate = prepare_play(
            snapshot,
            assets,
            scope,
            std::sync::Arc::new(Default::default()),
        )?;
        self.stop();
        self.play = Some(candidate);
        self.collect_script_diagnostics();
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(mut play) = self.play.take() {
            play.stop();
            self.console.collect(play.script_diagnostics());
        }
        self.selection.clear();
        self.picking.clear();
        self.drag_origin = None;
        self.preview_checkpoint = None;
        self.checkpoint_input = Default::default();
    }

    pub(crate) fn restart_play(&mut self) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        self.play.as_mut().context("请先运行场景")?.restart()?;
        self.collect_script_diagnostics();
        self.selection.clear();
        self.picking.clear();
        self.drag_origin = None;
        self.status = "已重开本次运行，作者文档保持不变".into();
        Ok(())
    }

    pub(crate) fn collect_script_diagnostics(&mut self) {
        if let Some(play) = &self.play {
            self.console.collect(play.script_diagnostics());
        }
    }

    pub(super) fn process_scene_request(&mut self, assets: &mut AssetStore) {
        let Some(request) = self.play.as_mut().and_then(PlaySession::take_scene_request) else {
            return;
        };
        let target = request.target();
        let candidate = (|| -> Result<PlaySession> {
            let snapshot = assets.expand_scene(target)?;
            let resources = runtime_resources(&snapshot, assets)?;
            let mut candidate = self
                .play
                .as_ref()
                .unwrap()
                .prepare_scene(snapshot, resources, target)?;
            comet_application::extract(&candidate.shared_snapshot(), assets, &[])?;
            Ok(candidate)
        })();
        let play = self.play.as_mut().unwrap();
        match candidate.and_then(|candidate| play.accept_candidate(candidate)) {
            Ok(()) => {
                self.status = "已切换运行场景，作者文档保持不变".into();
            }
            Err(error) => {
                play.reject_scene_request(request, format!("{error:#}"));
            }
        }
        self.collect_script_diagnostics();
    }
}

pub(super) fn prepare_play(
    snapshot: SceneSnapshot,
    assets: &mut AssetStore,
    scope: Option<AssetId>,
    progress: std::sync::Arc<comet_core::ProgressValues>,
) -> Result<PlaySession> {
    let resources = runtime_resources(&snapshot, assets)?;
    let mut candidate = match scope {
        Some(scene) => PlaySession::start_with_progress(snapshot, resources, scene, progress)?,
        None => PlaySession::start_with_resources(snapshot, resources)?,
    };
    // Initialization can change transforms; validate that result before replacing the old world.
    comet_application::extract(&candidate.shared_snapshot(), assets, &[])?;
    Ok(candidate)
}

fn runtime_resources(
    snapshot: &SceneSnapshot,
    assets: &mut AssetStore,
) -> Result<comet_runtime::SceneResources> {
    comet_application::extract(snapshot, assets, &[])?;
    assets.scene_resources(snapshot)
}
