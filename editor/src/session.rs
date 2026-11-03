use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext, ViewRequest};
use comet_asset::AssetStore;
use comet_asset_pipeline::{SourceCatalog, SourceMeta};
use comet_core::AssetId;
use comet_core::{EntityId, WorldId};
use comet_editor_core::Document;
use comet_render::{Camera, FrameFeedback};
use comet_runtime::PlaySession;
use comet_scene::{
    Character, ContentSource, EntityData, FieldOverride, PhysicsBody, PrefabResolver,
    SceneSnapshot,
    schema::{FieldKey, FieldValue},
};
use std::path::PathBuf;
pub struct Editor {
    pub(crate) documents: crate::documents::DocumentWorkflow,
    pub document: Document,
    pub play: Option<PlaySession>,
    pub selected: Option<EntityId>,
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
    pub drag_origin: Option<[f32; 3]>,
    pub gpu_ms: f64,
    pub export: crate::export::ExportController,
    pub project: crate::project::ProjectPanel,
    pub(crate) sources: SourceCatalog,
    source_revision: String,
    pub(crate) resource_selection: Option<AssetId>,
}
struct SnapshotCache {
    world: WorldId,
    document_revision: u64,
    products_revision: String,
    snapshot: std::sync::Arc<SceneSnapshot>,
}
impl Editor {
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
        let snapshot = std::sync::Arc::new(self.document.shared_source().expand(assets)?);
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
    pub fn start(&mut self, assets: &mut AssetStore) -> Result<()> {
        self.commit()?;
        let snapshot = self.document.source().expand(assets)?;
        comet_application::extract(&snapshot, assets, None)?;
        let candidate = PlaySession::start(snapshot)?;
        self.play = Some(candidate);
        self.selected = None;
        Ok(())
    }
    pub fn stop(&mut self) {
        self.play = None;
        self.selected = None;
        self.drag_origin = None;
    }
    pub fn commit(&mut self) -> Result<()> {
        if self.document.is_editing() {
            self.document.commit()?;
        }
        Ok(())
    }
    pub fn cancel(&mut self) -> Result<()> {
        if self.document.is_editing() {
            self.document.cancel()?;
        }
        self.drag_origin = None;
        Ok(())
    }
    pub fn edit(
        &mut self,
        id: EntityId,
        key: FieldKey,
        value: FieldValue,
        assets: &mut AssetStore,
    ) -> Result<()> {
        if let Some(play) = &mut self.play {
            return play.set_field(&[id], key, value);
        }
        if !self.document.is_editing() {
            self.document.begin()?;
        }
        let source = self.document.shared_source();
        if source.base.entities.iter().any(|e| e.id == id) {
            return self.document.preview_field(&[id], key, value);
        }
        for instance in &source.instances {
            let prefab = assets.prefab(instance.prefab)?;
            for entity in prefab.base.entities {
                if entity.id.in_instance(instance.id) == id {
                    return self.document.preview_override(
                        instance.id,
                        FieldOverride {
                            entity: entity.id,
                            field: key,
                            value,
                        },
                        assets,
                    );
                }
            }
        }
        anyhow::bail!("此对象暂不支持嵌套结构编辑")
    }
    pub fn edit_physics(
        &mut self,
        id: EntityId,
        body: Option<PhysicsBody>,
        character: Option<Character>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(self.play.is_none(), "请先停止运行，再修改物理组件");
        ensure!(!self.export.is_running(), "请等待导出完成，再修改物理组件");
        let began = !self.document.is_editing();
        if began {
            self.document.begin()?;
        }
        let result = self
            .document
            .preview_physics(id, body, character, assets)
            .context("无法更新物理组件，请检查对象层级、缩放及角色配置");
        if result.is_err() && began {
            self.document.cancel()?;
        }
        result
    }
    pub fn save(&mut self, assets: &mut AssetStore) -> Result<()> {
        ensure!(self.play.is_none(), "请先停止运行，再保存作者场景");
        self.commit()?;
        comet_application::extract(&self.document.source().expand(assets)?, assets, None)?;
        let path = self.paths.resolve_asset_path(&self.source)?;
        self.document.save(&path)?;
        self.status = "作者场景已保存".into();
        Ok(())
    }
    pub(crate) fn create_scene_source(&self, path: &std::path::Path) -> Result<PathBuf> {
        comet_asset_pipeline::create_scene(&self.paths, path)
    }
    pub(crate) fn switch_scene(&mut self, path: PathBuf, assets: &mut AssetStore) -> Result<()> {
        ensure!(!self.export.is_running(), "请等待导出完成，再切换场景");
        let (mut candidate, mut candidate_assets) =
            open(self.paths.root().to_owned(), self.products.clone(), path)?;
        let snapshot = candidate.shared_snapshot(&mut candidate_assets)?;
        comet_application::extract(&snapshot, &mut candidate_assets, None)?;
        let status = format!(
            "已打开场景：{}",
            candidate
                .source
                .strip_prefix(self.paths.assets())?
                .display()
        );
        self.stop();
        self.export.select_scene(candidate.export.entry_scene());
        self.document = candidate.document;
        self.source = candidate.source;
        self.project = candidate.project;
        self.sources = candidate.sources;
        self.source_revision = candidate.source_revision;
        self.resource_selection = None;
        self.camera = Camera::default();
        self.snapshot_cache = candidate.snapshot_cache;
        self.last_snapshot = snapshot;
        self.error = None;
        self.status = status;
        *assets = candidate_assets;
        Ok(())
    }
    pub(crate) fn begin_export(
        &mut self,
        assets: &mut AssetStore,
        action: crate::export::ExportAction,
    ) -> Result<()> {
        ensure!(!self.export.is_running(), "已有导出任务正在进行");
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
        self.export
            .start(&self.paths, &self.products, &self.source, action)
    }
    pub fn selected_entity(&self) -> Option<EntityData> {
        self.selected.and_then(|id| {
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
    fn request_close(&mut self) -> bool {
        self.request_editor_close()
    }
    fn close_ready(&self) -> bool {
        self.documents.approved
    }
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        input: &FrameContext,
    ) -> Result<ViewRequest> {
        self.export.poll(assets)?;
        if self.source_revision != assets.revision() {
            match SourceCatalog::read(&self.paths, assets.index()) {
                Ok(sources) => self.sources = sources,
                Err(error) => {
                    self.sources = SourceCatalog::default();
                    self.error = Some(format!("资源目录更新失败：{error:#}"));
                }
            }
            self.source_revision = assets.revision().to_owned();
            self.resource_selection = None;
        }
        self.target_generation = input.target_generation;
        let previous_world = self.world();
        let previous_play = self.play.as_ref().map(PlaySession::world_id);
        let previous_position = self
            .play
            .as_ref()
            .and_then(PlaySession::player_character)
            .map(|c| c.position);
        let active = input.active && !self.documents.is_blocking();
        if let Some(play) = &mut self.play {
            play.synchronize_host(active, input.activity_changed);
        }
        self.last_snapshot = self.shared_snapshot(assets)?;
        let mut view = None;
        let busy = self.export.is_running() || self.documents.is_blocking();
        ui.add_enabled_ui(!busy, |ui| {
            view = Some(crate::ui::show(self, ui, assets, active));
        });
        let (viewport, pick, actions) = view.context("editor UI")??;
        if !busy {
            self.project
                .show(ui.ctx(), &self.paths, assets, &self.sources);
        }
        if !self.documents.pending
            && !self.export.is_running()
            && let Some(action) = self.documents.show_browser(ui.ctx(), &self.sources)
        {
            self.request_document_action(action);
        }
        if !self.documents.is_blocking()
            && let Some(action) = self.export.show(ui.ctx(), &self.sources)
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
            play.advance_with_input(dt, actions)?;
            comet_application::follow_target(
                &mut self.camera,
                previous_position,
                play.player_character().map(|c| c.position),
            );
        }
        self.show_document_confirmation(ui.ctx(), assets);
        let snapshot = self.shared_snapshot(assets)?;
        self.last_snapshot = snapshot.clone();
        Ok(ViewRequest {
            world: self.world(),
            snapshot,
            viewport,
            camera: self.camera,
            selected: self.selected,
            pick: if self.world() == previous_world {
                pick
            } else {
                None
            },
            shadows: self.shadows,
        })
    }
    fn feedback(&mut self, feedback: &FrameFeedback) {
        self.gpu_ms = feedback.gpu_ms;
        if let Some(pick) = &feedback.pick
            && pick.token.world == self.world()
            && pick.token.target_generation == self.target_generation
        {
            self.selected = pick
                .entity
                .filter(|id| self.last_snapshot.entities.iter().any(|e| e.id == *id));
        }
    }
    fn report(&mut self) -> serde_json::Value {
        serde_json::json!({"dirty":self.document.is_dirty(),"history":self.document.history_len(),"play_active":self.play.is_some(),"entities":self.last_snapshot.entities.len(),"selected":self.selected})
    }
}
pub(crate) fn open(
    project: PathBuf,
    products: PathBuf,
    source: PathBuf,
) -> Result<(Editor, AssetStore)> {
    let paths = comet_core::ProjectPaths::open(&project)?;
    let source = paths.resolve_asset_path(&source)?;
    ensure!(
        source.extension().is_some_and(|e| e == "scene"),
        "author scene must use .scene"
    );
    let document = Document::open_source(ContentSource::from_json(&std::fs::read(&source)?)?)?;
    comet_asset_pipeline::prepare(paths.root(), &products)?;
    let assets = AssetStore::open(&products)?;
    let metadata = SourceMeta::load(&source)?;
    let export = crate::export::ExportController::new(metadata.id)?;
    let project = crate::project::ProjectPanel::new(&paths)?;
    let sources = SourceCatalog::read(&paths, assets.index())?;
    let source_revision = assets.revision().to_owned();
    let editor = Editor {
        documents: Default::default(),
        document,
        play: None,
        selected: None,
        camera: Camera::default(),
        products: comet_core::resolve_destination(&products)?,
        source,
        error: None,
        status: "就绪".into(),
        shadows: true,
        last_snapshot: std::sync::Arc::new(SceneSnapshot::default()),
        snapshot_cache: None,
        paths,
        target_generation: 0,
        drag_origin: None,
        gpu_ms: 0.,
        export,
        project,
        sources,
        source_revision,
        resource_selection: None,
    };
    Ok((editor, assets))
}

#[cfg(test)]
#[path = "../tests/unit/session.rs"]
mod tests;
