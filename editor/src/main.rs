mod animation;
mod audio;
mod components;
mod console;
mod documents;
mod export;
mod fields;
mod jobs;
mod joints;
mod models;
mod navigation;
mod objects;
mod picking;
mod preferences;
mod project;
mod resources;
mod scripts;
mod selection;
mod session;
#[cfg(test)]
mod test_support;
mod transform;
mod ui;
mod validation;
use anyhow::{Context, Result, ensure};
use comet_application::HostOptions;
use std::path::PathBuf;

fn main() -> Result<()> {
    let identity = comet_asset_data::package::RuntimeIdentity::for_build(
        env!("COMET_PROGRAM_BUILD_ID"),
        env!("COMET_PROGRAM_BUILD_TARGET"),
    );
    let mut options = HostOptions::new("Comet · 场景编辑器");
    options.icon = Some(
        include_bytes!("../resources/icons/editor.png")
            .as_slice()
            .into(),
    );
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
            "--app-template" => {
                template = Some(PathBuf::from(
                    args.next().context("--app-template requires a manifest")?,
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
    let (mut editor, mut assets) = session::open(project, products, source)?;
    if let Some(template) = template {
        match comet_asset_pipeline::AppTemplate::open(
            &template,
            &std::env::current_exe()?,
            &identity,
        ) {
            Ok(template) => editor.export.template = Some(template),
            Err(error) => editor.export.template_error = Some(format!("{error:#}")),
        }
    }
    if let Some(output) = export_game {
        editor.begin_export(
            &mut assets,
            crate::export::ExportAction {
                entry: editor
                    .export
                    .entry_scene()
                    .context("项目中没有可导出的启动场景")?,
                additional: include_scenes,
                output,
                run: false,
            },
        )?;
        let output = editor.wait_export(&mut assets)?;
        let report = serde_json::json!({"status":"passed","root":output.root,"executable":output.executable,"products":output.products});
        if let Some(path) = options.report {
            comet_core::atomic_write(&path, &serde_json::to_vec_pretty(&report)?)?;
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if !options.headless && !options.smoke {
        editor.enable_recents();
        editor.workspace.enable_user_state();
    }
    comet_application::run(options, assets, editor)
}
