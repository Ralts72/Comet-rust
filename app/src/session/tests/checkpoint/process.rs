use super::*;
use comet_core::{Checkpoint, GameId, WorldId, digest};
use std::{path::Path, process::Command};

const TEST: &str = "session::tests::checkpoint::process::checkpoint_survives_game_process_exit";
const PHASE: &str = "COMET_CHECKPOINT_TEST_PHASE";
const PRODUCTS: &str = "COMET_CHECKPOINT_TEST_PRODUCTS";
const DIRECTORY: &str = "COMET_CHECKPOINT_TEST_DIRECTORY";
const GAME: &str = "COMET_CHECKPOINT_TEST_GAME";
const FIRST: &str = "COMET_CHECKPOINT_TEST_FIRST";
const SECOND: &str = "COMET_CHECKPOINT_TEST_SECOND";

fn child(phase: &str) -> Result<()> {
    let products = PathBuf::from(std::env::var_os(PRODUCTS).context("child products")?);
    let directory =
        PathBuf::from(std::env::var_os(DIRECTORY).context("child checkpoint directory")?);
    let game: GameId = std::env::var(GAME)?.parse()?;
    let first: AssetId = std::env::var(FIRST)?.parse()?;
    let second: AssetId = std::env::var(SECOND)?.parse()?;
    assert!(products.is_absolute() && directory.is_absolute());
    let mut assets = AssetStore::open(&products)?;
    let mut app = app(&mut assets, first, Some(BTreeSet::from([first, second])))?;
    app.checkpoints = Ok(crate::storage::Store::directory(game, directory.clone())?);
    assert_eq!(app.scene, first);
    assert_eq!(app.play.scene_asset(), Some(first));
    assert_eq!(app.play.progress_snapshot().entries.len(), 1);
    assert_eq!(
        app.play.hud_frame().unwrap().labels[0].text.as_ref(),
        "fresh"
    );
    let file = directory.join(game.to_string()).join("checkpoint.json");
    match phase {
        "save" => {
            assert!(
                app.checkpoints
                    .as_ref()
                    .unwrap()
                    .load_checkpoint()?
                    .is_none()
            );
            app.process_scene_request(&mut assets);
            assert_eq!(app.scene, second);
            assert_eq!(app.play.scene_asset(), Some(second));
            assert_eq!(app.play.progress_snapshot().entries.len(), 2);
            app.save_checkpoint()?;
            let checkpoint = app
                .checkpoints
                .as_ref()
                .unwrap()
                .load_checkpoint()?
                .unwrap();
            assert_eq!(checkpoint.scene, second);
            assert_eq!(checkpoint.progress, *app.play.progress_snapshot());
            fs::write(
                directory.join("saved-process.json"),
                serde_json::to_vec(&json!({
                    "pid":std::process::id(),"world":app.play.world_id(),"scene":app.scene,
                    "progress_entries":checkpoint.progress.entries.len(),"file_digest":digest(&fs::read(file)?)
                }))?,
            )?;
        }
        "load" => {
            let before = fs::read(&file)?;
            let saved = Checkpoint::from_bytes(&before)?;
            saved.validate_for(game)?;
            let prior: Value =
                serde_json::from_slice(&fs::read(directory.join("saved-process.json"))?)?;
            let saved_world: WorldId = serde_json::from_value(prior["world"].clone())?;
            let entry_world = app.play.world_id();
            app.continue_checkpoint(&mut assets)?;
            assert_eq!(app.scene, second);
            assert_eq!(app.play.scene_asset(), Some(second));
            assert_ne!(app.play.world_id(), entry_world);
            assert_ne!(app.play.world_id(), saved_world);
            let values = app.play.progress_snapshot();
            assert_eq!(values.as_ref(), &saved.progress);
            assert_eq!(values.entries.len(), 2);
            for scene in [first, second] {
                let entries: Vec<_> = values
                    .entries
                    .iter()
                    .filter(|entry| entry.scene == scene)
                    .collect();
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].key, "done");
                assert_eq!(entries[0].value, ProgressValue::Bool(true));
            }
            assert_eq!(
                app.play.hud_frame().unwrap().labels[0].text.as_ref(),
                "restored"
            );
            assert!(
                app.play.take_scene_request().is_none(),
                "the first world's pending transition must be retired"
            );
            assert!(
                app.play
                    .script_diagnostics()
                    .iter()
                    .all(|entry| !entry.failed)
            );
            assert_eq!(
                fs::read(&file)?,
                before,
                "continuing never rewrites the saved slot"
            );
            fs::write(
                directory.join("loaded-process.json"),
                serde_json::to_vec(&json!({
                    "pid":std::process::id(),"world":app.play.world_id(),"scene":app.scene,
                    "progress_entries":values.entries.len(),"hud":"restored","file_digest":digest(&before)
                }))?,
            )?;
        }
        _ => anyhow::bail!("unknown checkpoint child phase"),
    }
    Ok(())
}

fn run_child(phase: &str, worlds: &Worlds, game: GameId, directory: &Path) -> Result<()> {
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", TEST, "--test-threads=1"])
        .env(PHASE, phase)
        .env(PRODUCTS, &worlds._products.0)
        .env(DIRECTORY, directory)
        .env(GAME, game.to_string())
        .env(FIRST, worlds.first.to_string())
        .env(SECOND, worlds.second.to_string())
        .output()?;
    anyhow::ensure!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
        "checkpoint {phase} child failed: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn checkpoint_survives_game_process_exit() -> Result<()> {
    if let Ok(phase) = std::env::var(PHASE) {
        return child(&phase);
    }
    let worlds = Worlds::new()?;
    let game = GameId::new();
    let directory = worlds._products.0.join("saved-games");
    run_child("save", &worlds, game, &directory)?;
    let file = directory.join(game.to_string()).join("checkpoint.json");
    let saved_bytes = fs::read(&file)?;
    run_child("load", &worlds, game, &directory)?;
    assert_eq!(fs::read(&file)?, saved_bytes);
    let saved: Value = serde_json::from_slice(&fs::read(directory.join("saved-process.json"))?)?;
    let loaded: Value = serde_json::from_slice(&fs::read(directory.join("loaded-process.json"))?)?;
    assert_ne!(saved["pid"], loaded["pid"]);
    assert_ne!(saved["world"], loaded["world"]);
    assert_eq!(saved["scene"], json!(worlds.second));
    assert_eq!(loaded["scene"], saved["scene"]);
    assert_eq!(saved["file_digest"], loaded["file_digest"]);
    let evidence = json!({
        "status":"passed","scope":"CPU App host and persistent checkpoint Store",
        "game_id":game,"saved_scene":saved["scene"],"loaded_scene":loaded["scene"],
        "progress_entries":2,"two_processes":true,"file_digest":digest(&saved_bytes),
        "saved_process":saved,"loaded_process":loaded
    });
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?;
    comet_core::atomic_write(
        &root.join("target/verification/checkpoint-process.json"),
        &serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(())
}
