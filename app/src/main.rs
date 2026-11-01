use anyhow::{Context, Result, ensure};
use comet_application::{Application, FrameContext, HostOptions, ViewRequest};
use comet_asset::AssetStore;
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
        self.play.advance(input.dt)?;
        let viewport = ui.max_rect();
        let response = ui.allocate_rect(viewport, egui::Sense::drag());
        comet_application::orbit(&mut self.camera, &response);
        Ok(ViewRequest {
            world: self.play.world_id(),
            snapshot: self.play.snapshot(),
            viewport,
            camera: self.camera,
            selected: None,
            pick: None,
            shadows: true,
        })
    }
    fn report(&mut self) -> serde_json::Value {
        serde_json::json!({"elapsed":self.play.elapsed(),"steps":self.play.steps(),"world":self.play.world_id()})
    }
}
fn main() -> Result<()> {
    let mut options = HostOptions::new("Comet Player");
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
        let manifest = directory.join("package.json");
        if manifest.is_file() {
            let package: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest)?)?;
            ensure!(
                package["version"] == 1,
                "unsupported Player package version"
            );
            if scene.is_none() {
                scene = Some(
                    package["entry_scene"]
                        .as_str()
                        .context("package entry_scene")?
                        .parse()?,
                );
            }
            products = Some(directory.join("products"));
        }
    }
    let mut assets = AssetStore::open(products.unwrap_or_else(|| PathBuf::from("products")))?;
    let scene = scene.context("pass --scene <asset UUID>")?;
    let play = PlaySession::start(assets.expand_scene(scene)?)?;
    comet_application::run(
        options,
        assets,
        Player {
            play,
            camera: Default::default(),
        },
    )
}
