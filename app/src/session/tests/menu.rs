use super::*;
use crate::settings::Settings;
use std::path::Path;

struct Game {
    _products: Products,
    assets: AssetStore,
    app: App,
    ctx: egui::Context,
    number: u64,
    output: Option<egui::FullOutput>,
}
impl Game {
    fn new(visible: bool) -> Result<Self> {
        let scene_id = AssetId::new();
        let program = AssetId::new();
        let mut owner = entity("game menu owner", Some(program));
        let audio = AssetId::new();
        owner["audio_source"] = serde_json::to_value(comet_scene::AudioSource {
            autoplay: true,
            looping: true,
            ..comet_scene::AudioSource::new(audio)
        })?;
        let mut products = vec![(
            program,
            script(
                "fn on_start(ctx) { this.ticks = ctx.progress(\"ticks\", 0); ctx.set_hud(this.ticks.to_string()); } fn on_fixed_update(ctx, dt) { this.ticks += 1; ctx.set_progress(\"ticks\", this.ticks); ctx.set_hud(this.ticks.to_string()); } fn on_stop(ctx) { ctx.log(\"stopped\"); }",
            ),
        )];
        products.push((
            audio,
            Product::Audio(comet_asset_data::AudioData {
                version: 1,
                sample_rate: 8000,
                channels: 1,
                samples: vec![0; 8000].into(),
            }),
        ));
        if visible {
            let mesh = AssetId::new();
            let material = AssetId::new();
            owner["model"] = json!({"mesh":mesh,"surface":{"material":material,"roughness":0.5}});
            let mut surface: comet_asset_data::product::MaterialData =
                serde_json::from_slice(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../tests/fixtures/prefab/assets/material.material"
                )))?;
            surface.base_color_texture = None;
            products.push((
                mesh,
                Product::Mesh(serde_json::from_slice(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../tests/fixtures/prefab/assets/mesh.mesh"
                )))?),
            ));
            products.push((material, Product::Material(surface)));
        }
        let mut content = scene(vec![owner])?;
        if let Product::Scene(source) = &mut content {
            source.base.version = 7;
        }
        products.push((scene_id, content));
        let (products, mut assets) = Products::new(products)?;
        let app = app(&mut assets, scene_id, Some(BTreeSet::from([scene_id])))?;
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        Ok(Self {
            _products: products,
            assets,
            app,
            ctx,
            number: 0,
            output: None,
        })
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> Result<ViewRequest> {
        let (view, output) = frame_output(
            &self.ctx,
            &mut self.app,
            &mut self.assets,
            self.number,
            true,
            false,
            events,
        )?;
        self.number += 1;
        self.output = Some(output);
        Ok(view)
    }
    fn key(&mut self, pressed: bool) -> Result<ViewRequest> {
        self.draw(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }])
    }
    fn open(&mut self) -> Result<()> {
        self.key(true)?;
        self.key(false)?;
        Ok(())
    }
    fn click(&mut self, label: &str) -> Result<ViewRequest> {
        self.draw(vec![])?;
        fn position(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| position(shape, label))
                }
                _ => None,
            }
        }
        let point = self
            .output
            .as_ref()
            .unwrap()
            .shapes
            .iter()
            .find_map(|shape| position(&shape.shape, label))
            .context("missing menu button")?;
        self.draw(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ])?;
        self.draw(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ])
    }
}

#[test]
fn menu_open_and_close_frames_never_advance_and_do_not_overwrite_the_world_pause() -> Result<()> {
    let mut game = Game::new(false)?;
    game.draw(vec![])?;
    let steps = game.app.play.steps();
    assert!(steps > 0);
    game.open()?;
    assert!(game.app.menu.is_open());
    assert_eq!(game.app.play.steps(), steps);
    assert!(!game.app.play.is_paused() && !game.app.play.is_suspended());
    assert!(game.app.audio_frame()?.unwrap().paused);
    game.draw(vec![egui::Event::Key {
        key: egui::Key::F5,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }])?;
    assert!(
        game.app
            .checkpoints
            .as_ref()
            .unwrap()
            .load_checkpoint()?
            .is_none()
    );
    game.click("返回游戏")?;
    assert!(!game.app.menu.is_open());
    assert_eq!(game.app.play.steps(), steps);
    assert!(game.app.audio_frame()?.unwrap().paused);
    game.draw(vec![])?;
    assert_eq!(game.app.play.steps(), steps + 1);
    assert!(!game.app.audio_frame()?.unwrap().paused);
    game.app.play.set_paused(true);
    game.open()?;
    game.click("返回游戏")?;
    game.draw(vec![])?;
    assert!(game.app.play.is_paused());
    assert_eq!(game.app.play.steps(), steps + 1);
    assert!(game.app.audio_frame()?.unwrap().paused);
    Ok(())
}

#[test]
fn menu_save_continue_and_exit_use_the_real_host_without_replaying_stop() -> Result<()> {
    let mut game = Game::new(false)?;
    game.draw(vec![])?;
    game.open()?;
    let steps = game.app.play.steps();
    let world = game.app.play.world_id();
    game.click("保存检查点")?;
    let saved = game
        .app
        .checkpoints
        .as_ref()
        .unwrap()
        .load_checkpoint()?
        .unwrap();
    assert_eq!(saved.progress.entries.len(), 1);
    assert!(game.app.menu.is_open());
    assert_eq!(game.app.play.steps(), steps);
    game.click("继续存档")?;
    let resumed = game.app.play.world_id();
    assert_ne!(resumed, world);
    assert!(!game.app.menu.is_open());
    assert_eq!(game.app.play.steps(), 0);
    assert_eq!(game.app.play.progress_snapshot().as_ref(), &saved.progress);
    assert!(game.app.audio_frame()?.unwrap().paused);
    game.draw(vec![])?;
    game.open()?;
    game.click("退出游戏")?;
    assert!(game.app.close_ready());
    assert!(game.app.audio_frame()?.is_none());
    game.app.request_close();
    game.app.report();
    assert_eq!(
        game.app
            .play
            .script_diagnostics()
            .iter()
            .filter(|entry| entry.world == resumed && entry.message == "stopped")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn settings_apply_changes_the_view_and_failed_storage_keeps_the_last_values_and_both_files()
-> Result<()> {
    let mut game = Game::new(false)?;
    game.open()?;
    game.click("设置")?;
    let draft = game.click("方向光阴影")?;
    assert!(draft.shadows && game.app.settings.shadows);
    let accepted = game.click("应用")?;
    assert!(!accepted.shadows && !game.app.settings.shadows);
    let id = game.app.checkpoints.as_ref().unwrap().game_id();
    let root = game._products.0.join("user-data");
    game.app.checkpoints = Ok(crate::storage::Store::directory(id, root.clone())?);
    let settings = Settings {
        master_volume: 0.25,
        shadows: false,
    };
    game.app.apply_settings(settings)?;
    game.app.save_checkpoint()?;
    let directory = root.join(id.to_string());
    let settings_bytes = fs::read(directory.join("settings.json"))?;
    let checkpoint_bytes = fs::read(directory.join("checkpoint.json"))?;
    let retained = root.with_extension("retained");
    fs::rename(&root, &retained)?;
    fs::write(&root, b"not a directory")?;
    assert!(game.app.apply_settings(Settings::default()).is_err());
    assert_eq!(game.app.settings, settings);
    assert_eq!(game.app.audio_master_gain(), 0.25);
    assert_eq!(
        fs::read(retained.join(id.to_string()).join("settings.json"))?,
        settings_bytes
    );
    assert_eq!(
        fs::read(retained.join(id.to_string()).join("checkpoint.json"))?,
        checkpoint_bytes
    );
    fs::remove_file(&root)?;
    fs::rename(&retained, &root)?;
    let reloaded = App::new(
        game.app.scene,
        &mut game.assets,
        game.app.allowed_scenes.clone(),
        Ok(crate::storage::Store::directory(id, root)?),
    )?;
    assert_eq!(reloaded.settings, settings);
    assert_eq!(reloaded.audio_master_gain(), 0.25);
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_pause_menu_draws_without_advancing_the_world_or_touching_player_data() -> Result<()> {
    let mut game = Game::new(true)?;
    game.open()?;
    assert_eq!(game.app.play.steps(), 0);
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/verification");
    fs::create_dir_all(&output)?;
    let mut options = comet_application::HostOptions::new("Comet 游戏");
    options.headless = true;
    options.smoke = true;
    options.report = Some(output.join("app-pause-menu.json"));
    options.capture = Some(output.join("app-pause-menu.png"));
    comet_application::run(options, game.assets, game.app)?;
    let report: Value = serde_json::from_slice(&fs::read(output.join("app-pause-menu.json"))?)?;
    assert_eq!(report["submitted_frames"], 8);
    assert_eq!(report["application"]["steps"], 0);
    assert_eq!(report["application"]["elapsed"], 0.);
    assert_eq!(report["application"]["menu_open"], true);
    assert_eq!(report["application"]["checkpoint"]["persistent"], false);
    assert!(
        report["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| frame["draw_count"] == 1)
    );
    Ok(())
}
