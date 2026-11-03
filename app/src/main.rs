use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext, HostOptions, ViewRequest};
use comet_asset::AssetStore;
use comet_asset_data::package::{PackageManifest, RuntimeIdentity};
use comet_runtime::PlaySession;
use std::path::PathBuf;
struct Player {
    play: PlaySession,
    camera: comet_application::Camera,
}
impl Application for Player {
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        _: &mut AssetStore,
        input: &FrameContext,
    ) -> Result<ViewRequest> {
        let viewport = ui.max_rect();
        let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
        if response.clicked() {
            response.request_focus();
        }
        comet_application::orbit(&mut self.camera, &response);
        let previous_position = self.play.player_character().map(|c| c.position);
        self.play
            .synchronize_host(input.active, input.activity_changed);
        self.play.advance_with_input(
            input.dt,
            comet_application::sample_actions(&response, input.active, self.camera),
        )?;
        comet_application::follow_target(
            &mut self.camera,
            previous_position,
            self.play.player_character().map(|c| c.position),
        );
        Ok(ViewRequest {
            world: self.play.world_id(),
            snapshot: self.play.shared_snapshot(),
            viewport,
            camera: self.camera,
            selected: None,
            pick: None,
            shadows: true,
        })
    }
    fn report(&mut self) -> serde_json::Value {
        serde_json::json!({"elapsed":self.play.elapsed(),"steps":self.play.steps(),"world":self.play.world_id(),"frame_steps":self.play.frame_steps(),"dropped_time":self.play.dropped_time(),"interpolation_alpha":self.play.interpolation_alpha(),"suspended":self.play.is_suspended(),"character":self.play.player_character().map(|c| serde_json::json!({"position":c.position,"grounded":c.grounded,"vertical_velocity":c.vertical_velocity}))})
    }
}
fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--template-info") {
        println!(
            "{}",
            serde_json::json!({"identity":RuntimeIdentity::current(),"debug":cfg!(debug_assertions)})
        );
        return Ok(());
    }
    let mut options = HostOptions::new("Comet 游戏");
    options.icon = Some(include_bytes!("../resources/icons/app.png"));
    let mut products = None;
    let mut scene = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--products" => {
                products = Some(PathBuf::from(
                    args.next().context("--products requires a path")?,
                ))
            }
            "--scene" => {
                scene = Some(
                    args.next()
                        .context("--scene requires an asset UUID")?
                        .parse::<comet_core::AssetId>()?,
                )
            }
            _ => ensure!(
                comet_application::parse_common(&mut options, &arg, &mut args)?,
                "unknown option {arg}"
            ),
        }
    }
    if products.is_none() {
        let executable = std::env::current_exe()?;
        let directory = executable.parent().context("Player directory")?;
        let resources = if directory.file_name().is_some_and(|n| n == "MacOS")
            && directory
                .parent()
                .is_some_and(|p| p.file_name().is_some_and(|n| n == "Contents"))
        {
            directory.parent().unwrap().join("Resources")
        } else {
            directory.to_owned()
        };
        let manifest: PackageManifest =
            serde_json::from_slice(&std::fs::read(resources.join("package.json")).context(
                "Player requires an exported package or explicit --products and --scene",
            )?)?;
        manifest.validate()?;
        if let Some(selected) = scene {
            ensure!(
                manifest.scenes.contains(&selected),
                "scene is not included in this package"
            );
        } else {
            scene = Some(manifest.entry_scene);
        }
        products = Some(resources.join("products"));
    }
    let mut assets = AssetStore::open(products.context("missing Player products")?)?;
    let scene = scene.context("pass --scene <asset UUID>")?;
    let play = PlaySession::start(assets.expand_scene(scene)?)?;
    let mut camera = comet_application::Camera::default();
    comet_application::follow_target(
        &mut camera,
        None,
        play.player_character().map(|c| c.position),
    );
    comet_application::run(options, assets, Player { play, camera })
}
