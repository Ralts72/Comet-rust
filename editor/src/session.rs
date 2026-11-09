use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext, ViewRequest};
use comet_asset::AssetStore;
use comet_asset_pipeline::SourceCatalog;
use comet_core::AssetId;
use comet_core::{EntityId, WorldId};
use comet_editor_core::Document;
use comet_render::{Camera, FrameFeedback};
use comet_runtime::PlaySession;
use comet_scene::{EntityData, SceneSnapshot};
use std::path::PathBuf;
mod checkpoint;
mod navigation;
mod play;
mod projects;
pub struct Editor {
    pub(crate) documents: crate::documents::DocumentWorkflow,
    pub(crate) directory_reader: crate::project::DirectoryReader,
    pub(crate) recent_projects: crate::project::RecentProjects,
    pub(crate) recent_warning: Option<String>,
    pub document: Document,
    pub play: Option<PlaySession>,
    pub(crate) selection: crate::selection::Selection,
    pub(crate) picking: crate::picking::Picking,
    pub camera: Camera,
    pub products: PathBuf,
    pub source: PathBuf,
    pub error: Option<String>,
    pub status: String,
    pub shadows: bool,
    pub last_snapshot: std::sync::Arc<SceneSnapshot>,
    snapshot_cache: Option<SnapshotCache>,
    paths: comet_core::ProjectPaths,
    pub target_generation: u64,
    pub(crate) drag_origin: Option<crate::transform::TransformDrag>,
    pub(crate) gizmo_tool: crate::transform::GizmoTool,
    pub(crate) transform_space: crate::transform::TransformSpace,
    pub(crate) translation_snap: SnapSettings,
    pub(crate) rotation_snap: SnapSettings,
    pub(crate) scale_snap: SnapSettings,
    pub gpu_ms: f64,
    pub export: crate::export::ExportController,
    pub project: crate::project::ProjectPanel,
    pub(crate) project_load: crate::project::ProjectLoad,
    product_locations: crate::project::ProductLocations,
    project_request: Option<projects::PendingProject>,
    pub(crate) sources: SourceCatalog,
    source_revision: String,
    pub(crate) resource_selection: Option<AssetId>,
    pub(crate) resource_build: crate::resources::ResourceBuild,
    pub(crate) navigation: crate::navigation::NavigationController,
    pub(crate) console: crate::console::ScriptConsole,
    checkpoint_input: comet_application::CheckpointInput,
    preview_checkpoint: Option<comet_core::Checkpoint>,
}
pub(crate) struct SnapSettings {
    pub enabled: bool,
    step: f32,
}
impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            step: 0.5,
        }
    }
}
impl SnapSettings {
    pub fn step(&self) -> f32 {
        self.step
    }
    pub fn set_step(&mut self, step: f32) -> Result<()> {
        ensure!(step.is_finite() && step > 0., "吸附步长必须是有限正数");
        self.step = step;
        Ok(())
    }
}
struct SnapshotCache {
    world: WorldId,
    document_revision: u64,
    products_revision: String,
    snapshot: std::sync::Arc<SceneSnapshot>,
}
// Editor snapshots come from ContentSource::expand or Scene::shared_snapshot, both UUID-sorted.
fn contains_entity(snapshot: &SceneSnapshot, id: EntityId) -> bool {
    snapshot
        .entities
        .binary_search_by_key(&id, |entity| entity.id)
        .is_ok()
}
impl Editor {
    pub(crate) fn is_busy(&self) -> bool {
        self.export.is_running()
            || self.resource_build.is_running()
            || self.project_load.is_running()
            || self.navigation.is_running()
    }
    fn verify_document_identity(&self) -> Result<()> {
        crate::project::verify_prefab_identity(
            &self.paths,
            &self.source,
            self.document.prefab_asset(),
        )
    }
    pub(crate) fn refresh_resources(&mut self) -> Result<()> {
        ensure!(!self.is_busy(), "请等待当前后台任务完成");
        ensure!(self.play.is_none(), "请先停止运行，再刷新资源");
        ensure!(!self.documents.is_blocking(), "请先完成当前文档操作");
        self.verify_document_identity()?;
        self.commit()?;
        self.resource_build.start(
            &self.paths,
            &self.products,
            self.document.world_id(),
            self.document.revision(),
            self.document.retained_sources().collect(),
            self.document.prefab_asset(),
        )
    }
    fn poll_resources(&mut self, assets: &mut AssetStore) {
        let Some(result) = self.resource_build.poll() else {
            return;
        };
        let result = result.and_then(|candidate| {
            self.verify_document_identity()
                .map_err(|error| format!("{error:#}"))?;
            if candidate.world != self.document.world_id()
                || candidate.document_revision != self.document.revision()
            {
                return Err("文档在资源刷新期间发生变化，请重新刷新".into());
            }
            let snapshot = std::sync::Arc::new(candidate.snapshot);
            self.selection.retain(|id| contains_entity(&snapshot, id));
            self.picking.clear();
            self.sources = candidate.sources;
            self.export.retain_scenes(&self.sources);
            self.source_revision = candidate.assets.revision().to_owned();
            self.resource_selection = None;
            self.snapshot_cache = Some(SnapshotCache {
                world: candidate.world,
                document_revision: candidate.document_revision,
                products_revision: self.source_revision.clone(),
                snapshot: snapshot.clone(),
            });
            self.last_snapshot = snapshot;
            *assets = candidate.assets;
            self.status = "资源刷新完成".into();
            Ok(())
        });
        if let Err(error) = result {
            self.resource_build.stage = "资源刷新失败".into();
            self.resource_build.failure = Some(error);
        }
    }
    fn shared_snapshot(
        &mut self,
        assets: &mut AssetStore,
    ) -> Result<std::sync::Arc<SceneSnapshot>> {
        if let Some(play) = &mut self.play {
            return Ok(play.shared_snapshot());
        }
        let world = self.document.world_id();
        let revision = self.document.revision();
        if let Some(cache) = &self.snapshot_cache
            && cache.world == world
            && cache.document_revision == revision
            && cache.products_revision == assets.revision()
        {
            return Ok(cache.snapshot.clone());
        }
        let snapshot = std::sync::Arc::new(self.document.expand(assets)?);
        self.snapshot_cache = Some(SnapshotCache {
            world,
            document_revision: revision,
            products_revision: assets.revision().to_owned(),
            snapshot: snapshot.clone(),
        });
        Ok(snapshot)
    }
    pub fn world(&self) -> WorldId {
        self.play
            .as_ref()
            .map_or(self.document.world_id(), PlaySession::world_id)
    }
    pub fn commit(&mut self) -> Result<()> {
        if self.document.is_editing() {
            self.document.commit()?;
        }
        self.drag_origin = None;
        Ok(())
    }
    pub fn cancel(&mut self) -> Result<()> {
        if self.document.is_editing() {
            self.document.cancel()?;
        }
        self.drag_origin = None;
        Ok(())
    }
    pub fn save(&mut self, assets: &mut AssetStore) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再保存文档");
        ensure!(self.play.is_none(), "请先停止运行，再保存作者文件");
        self.save_authoring(assets)
    }
    pub(crate) fn save_authoring(&mut self, assets: &mut AssetStore) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再保存文档");
        self.verify_document_identity()?;
        self.commit()?;
        crate::resources::validate_authoring(&self.document.expand(assets)?, assets)?;
        let path = self.paths.resolve_asset_path(&self.source)?;
        self.document.save(&path)?;
        self.status = format!("{}已保存", self.document_kind_label());
        Ok(())
    }
    pub(crate) fn create_scene_source(&self, path: &std::path::Path) -> Result<PathBuf> {
        comet_asset_pipeline::create_scene(&self.paths, path)
    }
    pub(crate) fn create_prefab_source(&self, path: &std::path::Path) -> Result<PathBuf> {
        comet_asset_pipeline::create_prefab(&self.paths, path)
    }
    pub(crate) fn switch_document(&mut self, path: PathBuf, assets: &mut AssetStore) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再切换文档");
        let candidate = crate::project::ProjectCandidate::load(
            self.paths.root().to_owned(),
            self.products.clone(),
            path,
        )?;
        self.accept_project_candidate(candidate, assets)
    }
    pub(crate) fn begin_export(
        &mut self,
        assets: &mut AssetStore,
        action: crate::export::ExportAction,
    ) -> Result<()> {
        ensure!(!self.is_busy(), "已有后台任务正在进行");
        let game_id = self
            .project
            .game_id()
            .context("项目缺少游戏身份，请重新打开项目")?;
        ensure!(
            comet_asset_pipeline::ProjectSettings::load(&self.paths)?.game_id == Some(game_id),
            "项目游戏身份已改变，请重新打开项目后再导出"
        );
        for scene in std::iter::once(action.entry).chain(action.additional.iter().copied()) {
            ensure!(
                assets
                    .index()
                    .products
                    .get(&scene)
                    .is_some_and(|entry| entry.kind == comet_asset_data::product::AssetKind::Scene),
                "导出入口及附加资源必须是场景"
            );
        }
        self.verify_document_identity()?;
        let template = self.export.template.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{}",
                self.export
                    .template_error
                    .as_deref()
                    .unwrap_or("缺少 App 模板，请重新运行 ./start.sh 准备程序")
            )
        })?;
        template.verify()?;
        let output = self
            .paths
            .export_destination(&action.output, &self.products)?;
        ensure!(
            std::fs::symlink_metadata(output)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "输出位置已存在，请选择一个新位置"
        );
        self.stop();
        self.commit()?;
        self.save(assets)?;
        self.export.start(
            &self.paths,
            &self.products,
            &self.source,
            action,
            self.document.retained_sources().collect(),
            crate::export::ExportIdentity {
                game_id,
                prefab_asset: self.document.prefab_asset(),
            },
        )
    }
    pub fn selected_entity(&self) -> Option<EntityData> {
        self.selection.active().and_then(|id| {
            self.last_snapshot
                .entities
                .iter()
                .find(|e| e.id == id)
                .cloned()
        })
    }
    pub fn notice(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.error = Some(format!("{e:#}"));
        }
    }
}
impl Application for Editor {
    fn audio_frame(&mut self) -> Result<Option<comet_application::AudioFrame>> {
        match self.play.as_mut() {
            Some(play) => play.audio_frame(),
            None => Ok(None),
        }
    }
    fn audio_error(&mut self, error: &str) {
        self.error = Some(format!("音频输出不可用：{error}"));
    }
    fn request_close(&mut self) -> bool {
        let approved = self.request_editor_close();
        if approved {
            self.stop();
        }
        approved
    }
    fn close_ready(&self) -> bool {
        self.documents.approved
    }
    fn view_error(&mut self, error: anyhow::Error) -> Result<ViewRequest> {
        if self.play.is_some() {
            self.stop();
            self.status = "运行因内容错误停止，作者场景保持不变".into();
        }
        self.picking.clear();
        self.error = Some(format!("场景暂时无法显示：{error:#}"));
        Ok(ViewRequest {
            world: self.document.world_id(),
            hud: None,
            snapshot: std::sync::Arc::new(SceneSnapshot::default()),
            // No scene or picking is submitted; UI rendering uses the whole framebuffer.
            viewport: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(1.)),
            camera: Camera::default(),
            selected: vec![],
            pick: None,
            shadows: false,
        })
    }
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        input: &FrameContext,
    ) -> Result<ViewRequest> {
        self.collect_script_diagnostics();
        self.directory_reader.poll();
        self.poll_project_transition(assets);
        self.poll_navigation(assets);
        self.poll_resources(assets);
        self.export.poll(assets);
        if self.source_revision != assets.revision() {
            match SourceCatalog::read(&self.paths, assets.index()) {
                Ok(sources) => self.sources = sources,
                Err(error) => {
                    self.sources = SourceCatalog::default();
                    self.error = Some(format!("资源目录更新失败：{error:#}"));
                }
            }
            self.export.retain_scenes(&self.sources);
            self.source_revision = assets.revision().to_owned();
            self.resource_selection = None;
        }
        if self.drag_origin.is_some() && (!input.active || input.activity_changed) {
            self.cancel_transform()?;
        }
        if input.activity_changed {
            crate::ui::cancel_viewport_gesture(ui.ctx());
        }
        self.target_generation = input.target_generation;
        let previous_world = self.world();
        let previous_play = self.play.as_ref().map(PlaySession::world_id);
        let previous_position = self
            .play
            .as_ref()
            .and_then(PlaySession::player_character)
            .map(|c| c.position);
        let active =
            input.active && !self.documents.is_blocking() && !self.project_load.is_running();
        if let Some(play) = &mut self.play {
            play.synchronize_host(active, input.activity_changed);
        }
        self.last_snapshot = self.shared_snapshot(assets)?;
        self.selection
            .retain(|id| contains_entity(&self.last_snapshot, id));
        let mut view = None;
        let busy = self.is_busy() || self.documents.is_blocking();
        ui.add_enabled_ui(!busy, |ui| {
            view = Some(crate::ui::show(self, ui, assets, active));
        });
        let view = view.context("editor UI")??;
        let viewport = view.viewport;
        let pick = view.pick;
        let actions = view.actions;
        if !self.is_busy() && !self.documents.is_blocking() {
            self.project
                .show(ui.ctx(), &self.paths, assets, &self.sources);
        }
        if !self.documents.pending
            && !self.is_busy()
            && let Some(action) =
                self.documents
                    .show_browser(ui.ctx(), &self.sources, &mut self.directory_reader)
        {
            self.request_document_action(action);
        }
        if !self.documents.is_blocking()
            && let Some(action) = self.export.show(
                ui.ctx(),
                &self.sources,
                self.resource_build.is_running() || self.project_load.is_running(),
            )
            && let Err(error) = self.begin_export(assets, action)
        {
            self.export.fail(format!("{error:#}"));
        }
        if let Some(play) = &mut self.play {
            if previous_play != Some(play.world_id()) {
                play.synchronize_host(active, true);
            }
            let dt = if previous_play == Some(play.world_id()) {
                input.dt
            } else {
                0.
            };
            let advanced = play.advance_with_input(dt, actions);
            self.console.collect(play.script_diagnostics());
            advanced?;
        }
        // UI may have opened a modal after host synchronization; leave scene requests pending.
        if input.active && !self.is_busy() && !self.documents.is_blocking() {
            self.process_scene_request(assets);
        }
        self.show_document_confirmation(ui.ctx(), assets);
        if self.documents.approved {
            self.stop();
        }
        let command = self.checkpoint_input.sample(
            &view.response,
            active
                && input.active
                && self.play.is_some()
                && !self.is_busy()
                && !self.documents.is_blocking(),
            input.activity_changed
                || previous_play != self.play.as_ref().map(PlaySession::world_id),
        );
        if let Some(command) = command {
            let result = self.preview_checkpoint_action(command, assets);
            self.notice(result);
        }
        if let Some(play) = &self.play {
            let same_world = previous_play == Some(play.world_id());
            if !same_world {
                self.selection.clear();
                self.picking.clear();
                self.drag_origin = None;
            }
            comet_application::follow_target(
                &mut self.camera,
                if same_world { previous_position } else { None },
                play.player_character().map(|c| c.position),
            );
        }
        let snapshot = self.shared_snapshot(assets)?;
        self.last_snapshot = snapshot.clone();
        self.selection.retain(|id| contains_entity(&snapshot, id));
        let pick = pick.filter(|pick| {
            self.world() == previous_world
                && !self.is_busy()
                && !self.documents.is_blocking()
                && !self.document.is_editing()
                && self.drag_origin.is_none()
                && pick.selection_generation == self.selection.generation()
        });
        if let Some(pick) = &pick {
            self.picking.request(
                input.number,
                self.world(),
                self.target_generation,
                pick.selection_generation,
                pick.mode,
            );
        }
        Ok(ViewRequest {
            world: self.world(),
            hud: self.play.as_ref().and_then(PlaySession::hud_frame),
            snapshot,
            viewport,
            camera: self.camera,
            selected: self.selection.ids(),
            pick: pick.map(|pick| pick.request),
            shadows: self.shadows,
        })
    }
    fn feedback(&mut self, feedback: &FrameFeedback) {
        self.gpu_ms = feedback.gpu_ms;
        let world = self.world();
        let result = self.picking.resolve(
            feedback,
            world,
            self.target_generation,
            self.selection.generation(),
        );
        if self.is_busy() || self.documents.is_blocking() || self.document.is_editing() {
            return;
        }
        if let Some((mode, entities)) = result {
            if entities
                .iter()
                .any(|id| !contains_entity(&self.last_snapshot, *id))
            {
                return;
            }
            self.selection.apply_pick(entities.iter().copied(), mode);
        }
    }
    fn report(&mut self) -> serde_json::Value {
        let hud = self.play.as_ref().and_then(PlaySession::hud_frame).map(|frame| serde_json::json!({"world": frame.world, "labels": frame.labels.iter().map(|label| serde_json::json!({"owner": label.owner, "text": label.text.as_ref()})).collect::<Vec<_>>()}));
        self.collect_script_diagnostics();
        serde_json::json!({
            "hud": hud,
            "dirty": self.document.is_dirty(),
            "history": self.document.history_len(),
            "prefab_asset": self.document.prefab_asset(),
            "play_active": self.play.is_some(),
            "play_scene": self.play.as_ref().and_then(PlaySession::scene_asset),
            "preview_checkpoint_saved": self.preview_checkpoint.is_some(),
            "progress_entries": self.play.as_ref().map(|play| play.progress_snapshot().entries.len()).unwrap_or(0),
            "entities": self.last_snapshot.entities.len(),
            "selected": self.selection.ids(),
            "script_diagnostics": self.console.report(),
            "error": self.error,
        })
    }
}
pub(crate) fn open(
    project: PathBuf,
    products: PathBuf,
    source: PathBuf,
) -> Result<(Editor, AssetStore)> {
    let mut candidate = crate::project::ProjectCandidate::load(project, products, source)?;
    candidate.verify_current()?;
    let product_locations =
        crate::project::ProductLocations::new(&candidate.paths, &candidate.products)?;
    let preferred_scene = candidate
        .prefab_asset()
        .is_none()
        .then_some(candidate.source_asset);
    let document = match candidate.prefab_asset() {
        Some(root) => Document::open_prefab(root, candidate.content, &mut candidate.assets)?,
        None => Document::open_source(candidate.content)?,
    };
    let world = document.world_id();
    let revision = document.revision();
    let assets = candidate.assets;
    let source_revision = assets.revision().to_owned();
    let mut export = crate::export::ExportController::new(preferred_scene)?;
    export.retain_scenes(&candidate.sources);
    let editor = Editor {
        documents: Default::default(),
        directory_reader: Default::default(),
        recent_projects: Default::default(),
        recent_warning: None,
        document,
        play: None,
        selection: Default::default(),
        picking: Default::default(),
        camera: Camera::default(),
        products: candidate.products,
        source: candidate.source,
        error: None,
        status: "就绪".into(),
        shadows: true,
        last_snapshot: candidate.snapshot.clone(),
        snapshot_cache: Some(SnapshotCache {
            world,
            document_revision: revision,
            products_revision: source_revision.clone(),
            snapshot: candidate.snapshot,
        }),
        paths: candidate.paths,
        target_generation: 0,
        drag_origin: None,
        gizmo_tool: Default::default(),
        transform_space: crate::transform::TransformSpace::World,
        translation_snap: Default::default(),
        rotation_snap: SnapSettings {
            enabled: false,
            step: 15.,
        },
        scale_snap: SnapSettings {
            enabled: false,
            step: 0.1,
        },
        gpu_ms: 0.,
        export,
        project: candidate.panel,
        project_load: Default::default(),
        product_locations,
        project_request: None,
        sources: candidate.sources,
        source_revision,
        resource_selection: None,
        resource_build: Default::default(),
        navigation: Default::default(),
        console: Default::default(),
        checkpoint_input: Default::default(),
        preview_checkpoint: None,
    };
    Ok((editor, assets))
}

#[cfg(test)]
mod tests;
