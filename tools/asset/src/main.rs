use anyhow::{Result, bail};
use comet_asset::AssetStore;
use comet_core::AssetId;
use std::path::Path;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["create-project", directory] => {
            let project = comet_asset_pipeline::create_project(Path::new(directory))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "project": project.root(),
                    "entry_scene": comet_asset_pipeline::DEFAULT_SCENE,
                }))?
            );
        }
        ["prepare", project, "--output", output] => {
            let report = comet_asset_pipeline::prepare(Path::new(project), Path::new(output))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        [
            "bake-navigation",
            project,
            source,
            "--products",
            products,
            "--output",
            output,
        ] => {
            let id = comet_asset_pipeline::bake_navigation_source(
                Path::new(project),
                Path::new(source),
                Path::new(products),
                Path::new(output),
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"id":id}))?
            );
        }
        ["inspect", products, id] => {
            let mut store = AssetStore::open(products)?;
            let id: AssetId = id.parse()?;
            let file = store.load(id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"id":id,"kind":file.payload.kind(),"dependencies":store.dependency_closure(id)?})
                )?
            );
        }
        ["expand", products, id, "--output", output] => {
            let mut store = AssetStore::open(products)?;
            let scene = store.expand_scene(id.parse()?)?;
            comet_core::atomic_write(Path::new(output), &scene.to_json()?)?;
            println!("Expanded {} entities to {output}", scene.entities.len());
        }
        [
            "pack",
            products,
            scene,
            "--game-id",
            game_id,
            "--app",
            app,
            "--output",
            output,
        ] => {
            comet_asset_pipeline::package(
                Path::new(products),
                game_id.parse()?,
                scene.parse()?,
                Path::new(app),
                Path::new(output),
            )?;
            println!("Packaged App and product closure in {output}");
        }
        _ => bail!(
            "usage: comet-assetc create-project <directory> | prepare <project> --output <products> | bake-navigation <project> <source.scene|source.prefab> --products <products> --output <assets-relative.nav> | inspect <products> <asset-uuid> | expand <products> <scene-uuid> --output <scene.json> | pack <products> <scene-uuid> --game-id <game-uuid> --app <binary> --output <new-directory>"
        ),
    }
    Ok(())
}
