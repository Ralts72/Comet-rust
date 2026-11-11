use crate::{
    menu::{Menu, MenuAction},
    settings::Settings,
};
use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext, ViewRequest};
use comet_asset::AssetStore;
use comet_runtime::PlaySession;
use std::{collections::BTreeSet, sync::Arc, time::Instant};
pub(super) struct App {
    play: PlaySession,
    camera: comet_application::Camera,
    scene: comet_core::AssetId,
    allowed_scenes: Option<BTreeSet<comet_core::AssetId>>,
    checkpoints: std::result::Result<crate::storage::Store, String>,
    checkpoint_input: comet_application::CheckpointInput,
    checkpoint_notice: Option<(String, Instant)>,
    menu: Menu,
    menu_blocks_frame: bool,
    settings: Settings,
    exit_requested: bool,
}
impl App {
    pub(super) fn new(
        scene: comet_core::AssetId,
        assets: &mut AssetStore,
        allowed_scenes: Option<BTreeSet<comet_core::AssetId>>,
        checkpoints: std::result::Result<crate::storage::Store, String>,
    ) -> Result<Self> {
        let snapshot = assets.expand_scene(scene)?;
        let play = prepare_play(scene, snapshot, assets, Arc::new(Default::default()))?;
        let mut camera = comet_application::Camera::default();
        comet_application::follow_target(
            &mut camera,
            None,
            play.player_character().map(|c| c.position),
        );
        let mut menu = Menu::default();
        let settings = match checkpoints
            .as_ref()
            .ok()
            .map(|store| store.load_settings())
            .transpose()
        {
            Ok(settings) => settings.flatten().unwrap_or_default(),
            Err(error) => {
                menu.feedback(Some(format!("读取设置失败，已使用默认设置：{error:#}")));
                Settings::default()
            }
        };
        Ok(Self {
            play,
            camera,
            scene,
            allowed_scenes,
            checkpoints,
            checkpoint_input: Default::default(),
            checkpoint_notice: None,
            menu,
            menu_blocks_frame: false,
            settings,
            exit_requested: false,
        })
    }
    fn process_scene_request(&mut self, assets: &mut AssetStore) {
        let Some(request) = self.play.take_scene_request() else {
            return;
        };
        let target = request.target();
        let result = (|| -> Result<()> {
            ensure!(
                self.allowed_scenes
                    .as_ref()
                    .is_none_or(|scenes| scenes.contains(&target)),
                "scene is not included in this package"
            );
            let snapshot = assets.expand_scene(target)?;
            let candidate = prepare_play(target, snapshot, assets, self.play.progress_snapshot())?;
            self.play.accept_candidate(candidate)
        })();
        match result {
            Ok(()) => self.scene = target,
            Err(error) => {
                self.play
                    .reject_scene_request(request, format!("{error:#}"));
            }
        }
    }

    fn save_checkpoint(&mut self) -> Result<()> {
        ensure!(
            self.play.scene_asset() == Some(self.scene),
            "当前运行场景缺少进度作用域"
        );
        let store = self
            .checkpoints
            .as_mut()
            .map_err(|error| anyhow::anyhow!(error.clone()))?;
        let checkpoint = comet_core::Checkpoint::new(
            store.game_id(),
            self.scene,
            (*self.play.progress_snapshot()).clone(),
        )?;
        store.save_checkpoint(&checkpoint)
    }

    fn continue_checkpoint(&mut self, assets: &mut AssetStore) -> Result<()> {
        let checkpoint = self
            .checkpoints
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))?
            .load_checkpoint()?
            .context("尚无存档，请先按 F5 保存")?;
        ensure!(
            self.allowed_scenes
                .as_ref()
                .is_none_or(|scenes| scenes.contains(&checkpoint.scene)),
            "存档关卡未包含在当前游戏包中"
        );
        let scene = checkpoint.scene;
        let snapshot = assets.expand_scene(scene)?;
        let candidate = prepare_play(scene, snapshot, assets, Arc::new(checkpoint.progress))?;
        if let Some(error) = candidate
            .script_diagnostics()
            .iter()
            .find(|entry| entry.failed)
        {
            anyhow::bail!(
                "存档场景初始化失败（对象 {}，脚本 {}）：{}",
                error.entity,
                error.asset,
                error.message
            );
        }
        self.play.accept_candidate(candidate)?;
        self.scene = scene;
        Ok(())
    }

    fn checkpoint_feedback(&mut self, result: Result<()>, success: &str) {
        let text = match result {
            Ok(()) => success.to_owned(),
            Err(error) => format!("{error:#}"),
        };
        self.checkpoint_notice = Some((text, Instant::now()));
    }

    fn apply_settings(&mut self, settings: Settings) -> Result<()> {
        settings.validate()?;
        self.checkpoints
            .as_mut()
            .map_err(|error| anyhow::anyhow!(error.clone()))?
            .save_settings(&settings)?;
        self.settings = settings;
        Ok(())
    }
}
impl Application for App {
    fn audio_master_gain(&self) -> f32 {
        self.settings.master_volume
    }
    fn audio_frame(&mut self) -> Result<Option<comet_application::AudioFrame>> {
        let mut frame = self.play.audio_frame()?;
        if let Some(frame) = &mut frame {
            frame.paused |= self.menu_blocks_frame;
        }
        Ok(frame)
    }
    fn request_close(&mut self) -> bool {
        self.play.stop();
        self.exit_requested = true;
        true
    }
    fn close_ready(&self) -> bool {
        self.exit_requested
    }
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        input: &FrameContext,
    ) -> Result<ViewRequest> {
        let viewport = ui.max_rect();
        let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        if response.clicked() {
            response.request_focus();
        }
        let previous_world = self.play.world_id();
        let previous_position = self.play.player_character().map(|c| c.position);
        let menu_frame = self.menu.show(
            &response,
            &self.settings,
            input.active,
            input.activity_changed,
        );
        let mut menu_changed = menu_frame.changed;
        match menu_frame.action {
            Some(MenuAction::Save) => {
                let result = self.save_checkpoint();
                match result {
                    Ok(()) => {
                        self.menu.feedback(None);
                        self.checkpoint_feedback(Ok(()), "存档已保存");
                    }
                    Err(error) => self.menu.feedback(Some(format!("{error:#}"))),
                }
            }
            Some(MenuAction::Continue) => {
                let result = self.continue_checkpoint(assets);
                match result {
                    Ok(()) => {
                        menu_changed |= self.menu.close();
                        self.menu.feedback(None);
                        self.checkpoint_feedback(Ok(()), "已继续存档，从保存关卡入口开始");
                    }
                    Err(error) => self.menu.feedback(Some(format!("{error:#}"))),
                }
            }
            Some(MenuAction::Apply(settings)) => {
                let result = self.apply_settings(settings);
                self.menu
                    .feedback(result.err().map(|error| format!("{error:#}")));
            }
            Some(MenuAction::Exit) => {
                self.request_close();
            }
            Some(MenuAction::Resume) | None => {}
        }
        if menu_changed && !self.menu.is_open() {
            response.request_focus();
        }
        self.menu_blocks_frame = menu_frame.blocked || menu_changed || self.exit_requested;
        self.play
            .synchronize_host(input.active, input.activity_changed || menu_changed);
        if !self.menu_blocks_frame && !self.exit_requested {
            comet_application::orbit(&mut self.camera, &response);
            self.play.advance_with_input(
                input.dt,
                comet_application::sample_actions(&response, input.active, self.camera),
            )?;
            if input.active {
                self.process_scene_request(assets);
            }
        }
        let command = self.checkpoint_input.sample(
            &response,
            input.active && !self.menu_blocks_frame,
            input.activity_changed || menu_changed || previous_world != self.play.world_id(),
        );
        match command {
            Some(comet_application::CheckpointAction::Save) => {
                let result = self.save_checkpoint();
                self.checkpoint_feedback(result, "存档已保存");
            }
            Some(comet_application::CheckpointAction::Continue) => {
                let result = self.continue_checkpoint(assets);
                self.checkpoint_feedback(result, "已继续存档，从保存关卡入口开始");
            }
            None => {}
        }
        comet_application::follow_target(
            &mut self.camera,
            if self.play.world_id() == previous_world {
                previous_position
            } else {
                None
            },
            self.play.player_character().map(|c| c.position),
        );
        ui.painter().text(
            viewport.right_bottom() - egui::vec2(18., 18.),
            egui::Align2::RIGHT_BOTTOM,
            if self.checkpoints.is_ok() {
                "Esc：菜单 · F5：保存 · F9：继续存档"
            } else {
                "Esc：暂停菜单"
            },
            egui::FontId::proportional(12.),
            egui::Color32::LIGHT_GRAY,
        );
        if let Some((text, since)) = &self.checkpoint_notice
            && since.elapsed().as_secs_f32() < 5.
        {
            egui::Area::new(egui::Id::unique("checkpoint-notice"))
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18., 18.))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_black_alpha(220))
                        .inner_margin(8.)
                        .show(ui, |ui| {
                            ui.set_max_width(420.);
                            ui.label(text);
                        });
                });
        }
        Ok(ViewRequest {
            world: self.play.world_id(),
            hud: self.play.hud_frame(),
            snapshot: self.play.shared_snapshot(),
            viewport,
            camera: self.camera,
            selected: vec![],
            pick: None,
            shadows: self.settings.shadows,
        })
    }
    fn report(&mut self) -> serde_json::Value {
        let hud =
            self.play.hud_frame().map(|frame| {
                let labels: Vec<_> = frame.labels.iter().map(|label| {
                serde_json::json!({"owner": label.owner, "text": label.text.as_ref()})
            }).collect();
                serde_json::json!({"world": frame.world, "labels": labels})
            });
        let snapshot = self.play.shared_snapshot();
        let (animations, animation_poses) = animation_report(&self.play, &snapshot);
        let progress_entries = self.play.progress_snapshot().entries.len();
        let mut joints = vec![];
        let navigation_agents: Vec<_> = snapshot
            .entities
            .iter()
            .filter_map(|entity| {
                entity
                    .navigation_agent
                    .as_ref()
                    .map(|agent| (entity, agent))
            })
            .map(|(entity, agent)| {
                serde_json::json!({
                    "entity": entity.id, "enabled": agent.enabled,
                    "position": self.play.character(entity.id).map(|character| character.position),
                    "status": self.play.navigation_status(entity.id).map(|status| status.as_str()),
                })
            })
            .collect();
        self.play.stop();
        let diagnostics: Vec<_> = self.play.script_diagnostics().iter().map(|entry| {
            serde_json::json!({
                "world":entry.world,"sequence":entry.sequence,"entity":entry.entity,"asset":entry.asset,
                "tick":entry.tick,"phase":entry.phase,"message":entry.message,"failed":entry.failed,
            })
        }).collect();
        for entity in &snapshot.entities {
            for (kind, label) in [
                (comet_runtime::JointKind::Point, "point"),
                (comet_runtime::JointKind::Hinge, "hinge"),
                (comet_runtime::JointKind::Slider, "slider"),
            ] {
                if let Some(status) = self.play.joint_status(entity.id, kind) {
                    let status = match status {
                        comet_runtime::JointStatus::Active => "active",
                        comet_runtime::JointStatus::Disabled => "disabled",
                        comet_runtime::JointStatus::Unbound => "unbound",
                    };
                    joints.push(serde_json::json!({
                        "entity":entity.id,"kind":label,"status":status,
                        "translation":entity.transform.translation,"rotation":entity.transform.rotation,
                    }));
                }
            }
        }
        serde_json::json!({
            "scene": self.scene,
            "animations": animations,
            "animation_poses": animation_poses,
            "hud": hud,
            "camera": {"eye": self.camera.eye.to_array(), "target": self.camera.target.to_array()},
            "elapsed": self.play.elapsed(),
            "steps": self.play.steps(),
            "world": self.play.world_id(),
            "frame_steps": self.play.frame_steps(),
            "dropped_time": self.play.dropped_time(),
            "interpolation_alpha": self.play.interpolation_alpha(),
            "suspended": self.play.is_suspended(),
            "character": self.play.player_character().map(|character| serde_json::json!({
                "position": character.position,
                "grounded": character.grounded,
                "vertical_velocity": character.vertical_velocity,
                "crouched": character.crouched,
            })),
            "script_diagnostics": diagnostics,
            "joints": joints,
            "navigation_agents": navigation_agents,
            "checkpoint": {"available": self.checkpoints.is_ok(),
                "persistent": self.checkpoints.as_ref().is_ok_and(crate::storage::Store::is_persistent),
                "last_message": self.checkpoint_notice.as_ref().map(|(text, _)| text)},
            "progress_entries": progress_entries,
            "settings": self.settings,
            "menu_open": self.menu.is_open(),
            "menu_blocks_frame": self.menu_blocks_frame,
        })
    }
}
fn animation_report(
    play: &PlaySession,
    snapshot: &comet_scene::SceneSnapshot,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut controllers = Vec::new();
    let mut poses = Vec::new();
    for owner in &snapshot.entities {
        let Some(animator) = &owner.animator else {
            continue;
        };
        let state = play.animation_state(owner.id);
        controllers.push(
            serde_json::json!({"entity":owner.id,"skeleton":animator.skeleton,
            "clip":state.as_ref().map(|state| state.clip),"controller":animator.controller,
            "state":state.as_ref().and_then(|state|state.state.as_deref()),
            "blend":state.as_ref().map(|state|state.blend),
            "time":state.as_ref().map(|state| state.time),
            "duration":state.as_ref().map(|state| state.duration),
            "playing":state.as_ref().map(|state| state.playing),
            "speed":state.as_ref().map(|state| state.speed),"bindings":animator.bindings.len()}),
        );
        for (source, reference) in &animator.bindings {
            let Some(comet_scene::EntityRef::Local { entity }) = reference else {
                continue;
            };
            // Runtime shared snapshots are UUID-sorted, independent of the source file's order.
            if let Ok(index) = snapshot
                .entities
                .binary_search_by_key(entity, |node| node.id)
            {
                let pose = &snapshot.entities[index].transform;
                poses.push(serde_json::json!({"source":source,"entity":entity,
                    "translation":pose.translation,"rotation":pose.rotation,"scale":pose.scale}));
            }
        }
    }
    (controllers, poses)
}

fn prepare_play(
    scene: comet_core::AssetId,
    snapshot: comet_scene::SceneSnapshot,
    assets: &mut AssetStore,
    progress: Arc<comet_core::ProgressValues>,
) -> Result<PlaySession> {
    comet_application::extract(&snapshot, assets, &[])?;
    let scripts = assets.script_sources(&snapshot)?;
    let data = assets.game_data_sources(&snapshot)?;
    let navigation = assets.navigation_data(&snapshot)?;
    let audio = assets.audio_sources(&snapshot)?;
    let libraries = assets.animation_sources(&snapshot)?;
    let mut candidate = PlaySession::start_with_progress(
        snapshot,
        comet_runtime::RuntimeResources {
            data,
            scripts,
            navigation,
            audio,
            skeletons: libraries.skeletons,
            animations: libraries.clips,
            controllers: libraries.controllers,
        },
        scene,
        progress,
    )?;
    comet_application::extract(&candidate.shared_snapshot(), assets, &[])?;
    Ok(candidate)
}

#[cfg(test)]
mod tests;
