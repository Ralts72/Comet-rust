use anyhow::{Context, Result, bail};
use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod metal;
#[cfg(not(target_os = "macos"))]
mod vulkan;

fn main() -> Result<()> {
    let mut output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/diagnostics/gpu");
    let mut capture = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => output = args.next().context("--output requires a directory")?.into(),
            "--capture" => capture = true,
            "--help" | "-h" => {
                println!("usage: comet-gpu-check [--output <directory>] [--capture]");
                return Ok(());
            }
            _ => bail!("unknown option {arg}"),
        }
    }
    std::fs::create_dir_all(&output)?;
    let output = output.canonicalize()?;
    #[cfg(target_os = "macos")]
    let report = metal::run(&output, capture)?;
    #[cfg(not(target_os = "macos"))]
    let report = {
        anyhow::ensure!(
            !capture,
            "GPU trace capture is supported by the Metal diagnostic only"
        );
        vulkan::run()?
    };
    let bytes = serde_json::to_vec_pretty(&report)?;
    std::fs::write(output.join("report.json"), &bytes)?;
    println!("{}", String::from_utf8(bytes)?);
    Ok(())
}
