use anyhow::{Context, Result, ensure};
use comet_application::HostOptions;
use comet_asset::AssetStore;
use comet_asset_data::package::{PackageManifest, RuntimeIdentity};
use session::App;
use std::path::PathBuf;
mod menu;
mod session;
mod settings;
mod storage;
fn main() -> Result<()> {
    let identity = RuntimeIdentity::for_build(
        env!("COMET_PROGRAM_BUILD_ID"),
        env!("COMET_PROGRAM_BUILD_TARGET"),
    );
    if std::env::args().nth(1).as_deref() == Some("--template-info") {
        println!(
            "{}",
            serde_json::json!({"identity":&identity,"debug":cfg!(debug_assertions)})
        );
        return Ok(());
    }
    let mut options = HostOptions::new("Comet 游戏");
    options.icon = Some(
        include_bytes!("../resources/icons/app.png")
            .as_slice()
            .into(),
    );
    let mut products = None;
    let mut scene = None;
    let mut allowed_scenes = None;
    let mut game_id = None;
    let mut data_directory = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                data_directory = Some(comet_core::resolve_destination(&PathBuf::from(
                    args.next().context("--data-dir requires a path")?,
                ))?)
            }
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
        let directory = executable.parent().context("App directory")?;
        let resources = if directory.file_name().is_some_and(|n| n == "MacOS")
            && directory
                .parent()
                .is_some_and(|p| p.file_name().is_some_and(|n| n == "Contents"))
        {
            directory.parent().unwrap().join("Resources")
        } else {
            directory.to_owned()
        };
        let manifest: PackageManifest = serde_json::from_slice(
            &std::fs::read(resources.join("package.json"))
                .context("App requires an exported package or explicit --products and --scene")?,
        )?;
        manifest.validate_for(&identity)?;
        game_id = Some(manifest.game_id);
        if let Some(expected) = &manifest.game_icon_sha256 {
            let path = resources.join("game-icon.png").canonicalize()?;
            ensure!(
                path.starts_with(resources.canonicalize()?),
                "游戏图标超出包目录"
            );
            let icon = std::fs::read(path)?;
            ensure!(comet_core::digest(&icon) == *expected, "游戏图标校验失败");
            options.icon = Some(icon.into());
        }
        if let Some(selected) = scene {
            ensure!(
                manifest.scenes.contains(&selected),
                "scene is not included in this package"
            );
        } else {
            scene = Some(manifest.entry_scene);
        }
        allowed_scenes = Some(manifest.scenes.into_iter().collect());
        products = Some(resources.join("products"));
    }
    let mut assets = AssetStore::open(products.context("missing App products")?)?;
    let scene = scene.context("pass --scene <asset UUID>")?;
    ensure!(
        game_id.is_some() || data_directory.is_none(),
        "--data-dir 需要导出游戏包中的游戏身份"
    );
    let checkpoints = match (game_id, data_directory) {
        (Some(id), Some(directory)) => storage::Store::directory(id, directory),
        (Some(id), None) if options.headless || options.smoke => Ok(storage::Store::memory(id)),
        (Some(id), None) => storage::Store::for_user(id),
        (None, None) => Err(anyhow::anyhow!(
            "独立产品运行没有游戏身份，请通过编辑器导出游戏后使用存档"
        )),
        (None, Some(_)) => unreachable!("game identity checked above"),
    }
    .map_err(|error| format!("存档不可用：{error:#}"));
    let app = App::new(scene, &mut assets, allowed_scenes, checkpoints)?;
    comet_application::run(options, assets, app)
}
