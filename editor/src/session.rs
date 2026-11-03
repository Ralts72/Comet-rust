use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext, HostOptions, ViewRequest};
use comet_asset::AssetStore;
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
    #[cfg(test)]
    pub fn snapshot(&mut self, assets: &mut AssetStore) -> Result<SceneSnapshot> {
        Ok(self.shared_snapshot(assets)?.as_ref().clone())
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
    fn begin_export(
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
                    .unwrap_or("缺少 Player 模板，请重新运行 ./start.sh 准备程序")
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
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        input: &FrameContext,
    ) -> Result<ViewRequest> {
        self.export.poll(assets)?;
        self.target_generation = input.target_generation;
        let previous_play = self.play.as_ref().map(PlaySession::world_id);
        let previous_position = self
            .play
            .as_ref()
            .and_then(PlaySession::player_character)
            .map(|c| c.position);
        if let Some(play) = &mut self.play {
            play.synchronize_host(input.active, input.activity_changed);
        }
        self.last_snapshot = self.shared_snapshot(assets)?;
        let mut view = None;
        let busy = self.export.is_running();
        ui.add_enabled_ui(!busy, |ui| {
            view = Some(crate::ui::show(self, ui, assets, input.active));
        });
        let (viewport, pick, actions) = view.context("editor UI")??;
        if let Some(action) = self.export.show(ui.ctx())
            && let Err(error) = self.begin_export(assets, action)
        {
            self.export.fail(format!("{error:#}"));
        }
        if let Some(play) = &mut self.play {
            if previous_play != Some(play.world_id()) {
                play.synchronize_host(input.active, true);
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
        let snapshot = self.shared_snapshot(assets)?;
        self.last_snapshot = snapshot.clone();
        Ok(ViewRequest {
            world: self.world(),
            snapshot,
            viewport,
            camera: self.camera,
            selected: self.selected,
            pick,
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
pub fn run() -> Result<()> {
    let mut options = HostOptions::new("Comet · 场景编辑器");
    options.icon = Some(include_bytes!("../resources/icons/editor.png"));
    let mut project = PathBuf::from("demo");
    let mut products = PathBuf::from("target/products/demo");
    let mut source = PathBuf::from("main.scene");
    let mut template = None;
    let mut export_game = None;
    let mut include_scenes = vec![];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--include-scene" => include_scenes.push(
                args.next()
                    .context("--include-scene requires a scene UUID")?
                    .parse::<comet_core::AssetId>()?,
            ),
            "--player-template" => {
                template = Some(PathBuf::from(
                    args.next()
                        .context("--player-template requires a manifest")?,
                ))
            }
            "--export-game" => {
                export_game = Some(PathBuf::from(
                    args.next()
                        .context("--export-game requires an output directory")?,
                ))
            }
            "--project" => project = args.next().context("--project requires a path")?.into(),
            "--products" => products = args.next().context("--products requires a path")?.into(),
            "--scene" => {
                source = args
                    .next()
                    .context("--scene requires an authoring filename")?
                    .into()
            }
            _ => {
                ensure!(
                    comet_application::parse_common(&mut options, &arg, &mut args)?,
                    "unknown option {arg}"
                );
            }
        }
    }
    let (mut editor, mut assets) = open(project, products, source)?;
    if let Some(template) = template {
        match comet_asset_pipeline::PlayerTemplate::open(&template, &std::env::current_exe()?) {
            Ok(template) => editor.export.template = Some(template),
            Err(error) => editor.export.template_error = Some(format!("{error:#}")),
        }
    }
    if let Some(output) = export_game {
        editor.begin_export(
            &mut assets,
            crate::export::ExportAction {
                entry: editor.export.entry_scene(),
                additional: include_scenes,
                output,
                run: false,
            },
        )?;
        let output = editor.export.wait(&mut assets)?;
        let report = serde_json::json!({"status":"passed","root":output.root,"executable":output.executable,"products":output.products});
        if let Some(path) = options.report {
            comet_core::atomic_write(&path, &serde_json::to_vec_pretty(&report)?)?;
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    comet_application::run(options, assets, editor)
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
    let metadata: comet_asset_pipeline::SourceMeta =
        serde_json::from_slice(&std::fs::read(format!("{}.meta", source.display()))?)?;
    let export = crate::export::ExportController::new(&paths, &assets, metadata.id)?;
    let editor = Editor {
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
    };
    Ok((editor, assets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use comet_render::{PickFeedback, ViewToken};
    use comet_scene::schema;
    fn fixture() -> Result<(tempfile::TempDir, Editor, AssetStore)> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        std::fs::create_dir_all(project.join("assets"))?;
        for file in std::fs::read_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/gltf/assets"),
        )? {
            let file = file?;
            if file.file_name() != ".assetc.lock" {
                std::fs::copy(file.path(), project.join("assets").join(file.file_name()))?;
            }
        }
        let (editor, assets) = open(project, temp.path().join("products"), "main.scene".into())?;
        Ok((temp, editor, assets))
    }
    #[test]
    fn authored_physics_survives_save_reopen_and_cannot_change_during_play() -> Result<()> {
        use comet_scene::{BodyMotion, ColliderShape};
        let (temp, mut editor, mut assets) = fixture()?;
        let mut source = editor.document.source();
        let mut entity = EntityData::new("新角色");
        let id = entity.id;
        entity.transform.translation = [0., 3., 0.];
        source.base.entities.push(entity);
        editor.document = Document::open_source(source)?;
        let mut body = PhysicsBody {
            motion: BodyMotion::Kinematic,
            shape: ColliderShape::Capsule {
                half_height: 0.5,
                radius: 0.3,
            },
            ..Default::default()
        };
        editor.edit_physics(
            id,
            Some(body.clone()),
            Some(Character::default()),
            &mut assets,
        )?;
        editor.commit()?;
        for radius in [0.4, 0.5, 0.6] {
            body.shape = ColliderShape::Capsule {
                half_height: 0.5,
                radius,
            };
            editor.edit_physics(
                id,
                Some(body.clone()),
                Some(Character::default()),
                &mut assets,
            )?;
        }
        editor.commit()?;
        assert_eq!(editor.document.history_len(), 2);
        let configured = editor.document.shared_source();
        let mut invalid = body;
        invalid.motion = BodyMotion::Dynamic;
        assert!(
            editor
                .edit_physics(id, Some(invalid), Some(Character::default()), &mut assets)
                .is_err()
        );
        assert!(!editor.document.is_editing());
        assert_eq!(editor.document.shared_source(), configured);
        editor.save(&mut assets)?;
        let (mut reopened, mut rebuilt) = open(
            temp.path().join("project"),
            temp.path().join("rebuilt"),
            "main.scene".into(),
        )?;
        assert_eq!(reopened.document.shared_source(), configured);
        reopened.start(&mut rebuilt)?;
        let world = reopened.world();
        assert!(reopened.edit_physics(id, None, None, &mut rebuilt).is_err());
        assert_eq!(reopened.world(), world);
        assert!(!reopened.document.is_editing());
        let play = reopened.play.as_mut().unwrap();
        for _ in 0..30 {
            play.advance(1. / 60.)?;
        }
        assert!(play.character(id).unwrap().position[1] < 3.);
        reopened.stop();
        assert_eq!(reopened.document.shared_source(), configured);
        assert!(!reopened.document.is_dirty());
        editor.edit_physics(id, None, None, &mut assets)?;
        editor.commit()?;
        editor.document.undo()?;
        assert_eq!(editor.document.shared_source(), configured);
        assert!(!editor.document.is_dirty());
        Ok(())
    }
    #[test]
    fn model_edit_save_reopen_history_and_play_stop_isolation() -> Result<()> {
        let (_temp, mut editor, mut assets) = fixture()?;
        let snapshot = editor.snapshot(&mut assets)?;
        let id = snapshot
            .entities
            .iter()
            .find(|e| e.name == "Amber cube")
            .unwrap()
            .id;
        for x in [0.2, 0.5, 0.8] {
            editor.edit(
                id,
                schema::TRANSLATION,
                FieldValue::Vec3([x, 0.7, 0.]),
                &mut assets,
            )?;
        }
        editor.commit()?;
        assert_eq!(editor.document.history_len(), 1);
        let edited = editor.snapshot(&mut assets)?;
        editor.document.undo()?;
        assert_eq!(editor.snapshot(&mut assets)?, snapshot);
        editor.document.redo()?;
        assert_eq!(editor.snapshot(&mut assets)?, edited);
        editor.save(&mut assets)?;
        assert!(!editor.document.is_dirty());
        let saved = editor.document.source();
        assert_eq!(Document::load(&editor.source)?.source(), saved);
        let history = editor.document.history_len();
        editor.start(&mut assets)?;
        let play_world = editor.world();
        assert_ne!(play_world, editor.document.world_id());
        editor.play.as_mut().unwrap().set_paused(true);
        editor.play.as_mut().unwrap().advance(10.)?;
        assert_eq!(editor.play.as_ref().unwrap().steps(), 0);
        editor.play.as_mut().unwrap().single_step()?;
        assert_eq!(editor.play.as_ref().unwrap().steps(), 1);
        editor.edit(
            id,
            schema::TRANSLATION,
            FieldValue::Vec3([9., 8., 7.]),
            &mut assets,
        )?;
        assert_ne!(editor.snapshot(&mut assets)?, edited);
        editor.stop();
        assert_eq!(editor.snapshot(&mut assets)?, edited);
        assert_eq!(editor.document.source(), saved);
        assert_eq!(editor.document.history_len(), history);
        assert!(!editor.document.is_dirty());
        editor.start(&mut assets)?;
        assert_ne!(editor.world(), play_world);
        Ok(())
    }
    #[test]
    fn invalid_play_candidate_keeps_the_edit_owner_and_history() -> Result<()> {
        let (_temp, mut editor, mut assets) = fixture()?;
        let snapshot = editor.snapshot(&mut assets)?;
        let id = snapshot
            .entities
            .iter()
            .find(|e| e.model.is_some())
            .unwrap()
            .id;
        editor.edit(
            id,
            schema::MESH,
            FieldValue::Asset(comet_core::AssetId::new()),
            &mut assets,
        )?;
        editor.commit()?;
        let source = editor.document.source();
        let history = editor.document.history_len();
        assert!(editor.start(&mut assets).is_err());
        assert!(editor.play.is_none());
        assert_eq!(editor.world(), editor.document.world_id());
        assert_eq!(editor.document.source(), source);
        assert_eq!(editor.document.history_len(), history);
        editor.document.undo()?;
        editor.start(&mut assets)?;
        Ok(())
    }
    #[test]
    fn stale_world_target_and_removed_entity_pick_results_are_rejected() -> Result<()> {
        let (_temp, mut editor, mut assets) = fixture()?;
        editor.last_snapshot = editor.shared_snapshot(&mut assets)?;
        let entity = editor.last_snapshot.entities[0].id;
        editor.target_generation = 7;
        let mut feedback = FrameFeedback {
            number: 1,
            token: ViewToken {
                world: editor.world(),
                view: 1,
                target_generation: 7,
            },
            presented: true,
            backend: "test".into(),
            worker_thread: "test".into(),
            draw_count: 5,
            gpu_ms: 0.,
            pick: None,
        };
        for token in [
            ViewToken {
                world: WorldId::new(),
                ..feedback.token
            },
            ViewToken {
                target_generation: 6,
                ..feedback.token
            },
        ] {
            feedback.pick = Some(PickFeedback {
                token,
                entity: Some(entity),
            });
            editor.feedback(&feedback);
            assert!(editor.selected.is_none());
        }
        feedback.pick = Some(PickFeedback {
            token: feedback.token,
            entity: Some(EntityId::new()),
        });
        editor.feedback(&feedback);
        assert!(editor.selected.is_none());
        feedback.pick = Some(PickFeedback {
            token: feedback.token,
            entity: Some(entity),
        });
        editor.feedback(&feedback);
        assert_eq!(editor.selected, Some(entity));
        Ok(())
    }
    #[test]
    fn author_scene_paths_cannot_escape_and_save_rechecks_the_current_parent() -> Result<()> {
        let (temp, mut editor, mut assets) = fixture()?;
        let project = temp.path().join("project");
        let outside = temp.path().join("outside.scene");
        std::fs::write(&outside, editor.document.source().to_json()?)?;
        let bytes = std::fs::read(&outside)?;
        assert!(
            open(
                project.clone(),
                temp.path().join("bad-products"),
                "../outside.scene".into()
            )
            .is_err()
        );
        assert!(open(project, temp.path().join("bad-products"), outside.clone()).is_err());
        editor.source = outside.clone();
        assert!(editor.save(&mut assets).is_err());
        assert_eq!(std::fs::read(outside)?, bytes);
        Ok(())
    }
    #[test]
    fn expanded_snapshot_cache_tracks_document_products_and_play_worlds() -> Result<()> {
        use std::sync::Arc;
        let (temp, mut editor, mut assets) = fixture()?;
        let before = editor.shared_snapshot(&mut assets)?;
        for _ in 0..100 {
            assert!(Arc::ptr_eq(&before, &editor.shared_snapshot(&mut assets)?));
        }
        let source = temp.path().join("project/assets/showcase.gltf");
        let mut gltf: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
        gltf["nodes"][1]["translation"][0] = serde_json::json!(-2.);
        std::fs::write(source, serde_json::to_vec(&gltf)?)?;
        comet_asset_pipeline::prepare(editor.paths.root(), &editor.products)?;
        assets.refresh()?;
        let reloaded = editor.shared_snapshot(&mut assets)?;
        assert!(!Arc::ptr_eq(&before, &reloaded));
        assert_ne!(*before, *reloaded);
        editor.start(&mut assets)?;
        let play = editor.shared_snapshot(&mut assets)?;
        assert!(!Arc::ptr_eq(&reloaded, &play));
        editor.stop();
        assert!(Arc::ptr_eq(
            &reloaded,
            &editor.shared_snapshot(&mut assets)?
        ));
        Ok(())
    }
}
