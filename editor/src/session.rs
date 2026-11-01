use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext, HostOptions, ViewRequest};
use comet_asset::AssetStore;
use comet_core::{EntityId, WorldId};
use comet_editor_core::Document;
use comet_render::{Camera, FrameFeedback};
use comet_runtime::PlaySession;
use comet_scene::{
    ContentSource, EntityData, FieldOverride, PrefabResolver, SceneSnapshot,
    schema::{FieldKey, FieldValue},
};
use std::path::PathBuf;
pub struct Editor {
    pub document: Document,
    pub play: Option<PlaySession>,
    pub selected: Option<EntityId>,
    pub camera: Camera,
    pub project: PathBuf,
    pub products: PathBuf,
    pub source: PathBuf,
    pub error: Option<String>,
    pub status: String,
    pub shadows: bool,
    pub last_snapshot: SceneSnapshot,
    pub target_generation: u64,
    pub drag_origin: Option<[f32; 3]>,
    pub gpu_ms: f64,
}
impl Editor {
    pub fn world(&self) -> WorldId {
        self.play
            .as_ref()
            .map_or(self.document.world_id(), PlaySession::world_id)
    }
    pub fn snapshot(&mut self, assets: &mut AssetStore) -> Result<SceneSnapshot> {
        if let Some(play) = &mut self.play {
            Ok(play.snapshot())
        } else {
            self.document.source().expand(assets)
        }
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
        let source = self.document.source();
        if source.base.entities.iter().any(|e| e.id == id) {
            return self.document.preview_field(&[id], key, value);
        }
        for instance in source.instances {
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
        anyhow::bail!("Nested structural editing is not available for this entity")
    }
    pub fn save(&mut self, assets: &mut AssetStore) -> Result<()> {
        ensure!(
            self.play.is_none(),
            "stop Play before saving authoring content"
        );
        self.commit()?;
        comet_application::extract(&self.document.source().expand(assets)?, assets, None)?;
        self.document.save(&self.source)?;
        comet_asset_pipeline::prepare(&self.project, &self.products)?;
        assets.refresh()?;
        self.status = "Saved and rebuilt products".into();
        Ok(())
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
        self.target_generation = input.target_generation;
        self.last_snapshot = self.snapshot(assets)?;
        let (viewport, pick) = crate::ui::show(self, ui, assets)?;
        if let Some(play) = &mut self.play {
            play.advance(input.dt)?;
        }
        let snapshot = self.snapshot(assets)?;
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
    let mut options = HostOptions::new("Comet · Scene Editor");
    let mut project = PathBuf::from("demo");
    let mut products = PathBuf::from("target/products/demo");
    let mut source = PathBuf::from("main.scene");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
    let (editor, assets) = open(project, products, source)?;
    comet_application::run(options, assets, editor)
}
pub(crate) fn open(
    project: PathBuf,
    products: PathBuf,
    source: PathBuf,
) -> Result<(Editor, AssetStore)> {
    comet_asset_pipeline::prepare(&project, &products)?;
    let source = project.join("assets").join(source);
    let document = Document::open_source(ContentSource::from_json(&std::fs::read(&source)?)?)?;
    let assets = AssetStore::open(&products)?;
    let editor = Editor {
        document,
        play: None,
        selected: None,
        camera: Camera::default(),
        project,
        products,
        source,
        error: None,
        status: "Ready".into(),
        shadows: true,
        last_snapshot: SceneSnapshot::default(),
        target_generation: 0,
        drag_origin: None,
        gpu_ms: 0.,
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
        editor.last_snapshot = editor.snapshot(&mut assets)?;
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
}
