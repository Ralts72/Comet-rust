use super::*;
use comet_core::{ProgressValue, math::Vec3};
use comet_scene::schema;
use std::sync::Arc;
mod process;

struct Worlds {
    _products: Products,
    assets: AssetStore,
    first: AssetId,
    second: AssetId,
    owner: EntityId,
}
impl Worlds {
    fn new() -> Result<Self> {
        let first = AssetId::new();
        let second = AssetId::new();
        let a = AssetId::new();
        let b = AssetId::new();
        let next = entity("B progress owner", Some(b));
        let owner = serde_json::from_value(next["id"].clone())?;
        let start = "let old = ctx.progress(\"done\", false); ctx.set_hud(if old { \"restored\" } else { \"fresh\" }); ctx.set_progress(\"done\", true);";
        let (products, assets) = Products::new(vec![
            (first, scene(vec![entity("A progress owner", Some(a))])?),
            (second, scene(vec![next])?),
            (
                a,
                script(format!(
                    "fn on_start(ctx) {{ {start} ctx.request_scene(\"{second}\"); }}"
                )),
            ),
            (b, script(format!("fn on_start(ctx) {{ {start} }}"))),
        ])?;
        Ok(Self {
            _products: products,
            assets,
            first,
            second,
            owner,
        })
    }
    fn app(&mut self) -> Result<App> {
        let mut app = app(
            &mut self.assets,
            self.first,
            Some(BTreeSet::from([self.first, self.second])),
        )?;
        app.process_scene_request(&mut self.assets);
        assert_eq!(app.scene, self.second);
        Ok(app)
    }
}

#[test]
fn continuing_checks_the_scene_and_startup_before_replacing_the_world_or_saved_slot() -> Result<()>
{
    let mut worlds = Worlds::new()?;
    let mut app = worlds.app()?;
    assert_eq!(app.play.progress_snapshot().entries.len(), 2);
    app.save_checkpoint()?;
    let saved = app
        .checkpoints
        .as_ref()
        .unwrap()
        .load_checkpoint()?
        .unwrap();
    let bytes = saved.to_bytes()?;
    app.play.set_field(
        &[worlds.owner],
        schema::TRANSLATION,
        schema::FieldValue::Vec3(Vec3::splat(3.).to_array()),
    )?;
    app.play.set_paused(true);
    let world = app.play.world_id();
    let snapshot = app.play.shared_snapshot();
    let progress = app.play.progress_snapshot();
    app.allowed_scenes = Some(BTreeSet::from([worlds.first]));
    assert!(app.continue_checkpoint(&mut worlds.assets).is_err());
    assert_eq!(app.play.world_id(), world);
    assert!(Arc::ptr_eq(&snapshot, &app.play.shared_snapshot()));
    assert!(Arc::ptr_eq(&progress, &app.play.progress_snapshot()));
    assert_eq!(
        app.checkpoints
            .as_ref()
            .unwrap()
            .load_checkpoint()?
            .unwrap()
            .to_bytes()?,
        bytes
    );
    app.allowed_scenes.as_mut().unwrap().insert(worlds.second);
    let mut incompatible = saved.clone();
    incompatible
        .progress
        .entries
        .iter_mut()
        .find(|entry| entry.scene == worlds.second)
        .unwrap()
        .value = ProgressValue::Text("wrong game value type".into());
    app.checkpoints
        .as_mut()
        .unwrap()
        .save_checkpoint(&incompatible)?;
    let bad_bytes = incompatible.to_bytes()?;
    assert!(app.continue_checkpoint(&mut worlds.assets).is_err());
    assert_eq!(app.play.world_id(), world);
    assert!(Arc::ptr_eq(&snapshot, &app.play.shared_snapshot()));
    assert!(Arc::ptr_eq(&progress, &app.play.progress_snapshot()));
    assert_eq!(
        app.checkpoints
            .as_ref()
            .unwrap()
            .load_checkpoint()?
            .unwrap()
            .to_bytes()?,
        bad_bytes
    );
    app.checkpoints.as_mut().unwrap().save_checkpoint(&saved)?;
    app.continue_checkpoint(&mut worlds.assets)?;
    assert_ne!(app.play.world_id(), world);
    assert_eq!(app.scene, worlds.second);
    assert!(app.play.is_paused());
    assert_eq!(app.play.steps(), 0);
    assert_eq!(
        app.play.hud_frame().unwrap().labels[0].text.as_ref(),
        "restored"
    );
    assert_eq!(
        app.play.shared_snapshot().entities[0].transform.translation,
        [0.; 3]
    );
    assert_eq!(app.play.progress_snapshot().as_ref(), &saved.progress);
    app.play.restart()?;
    assert_eq!(app.play.progress_snapshot().as_ref(), &saved.progress);
    Ok(())
}

#[test]
fn paused_app_shortcuts_save_and_continue_once_and_never_open_a_user_directory() -> Result<()> {
    let mut worlds = Worlds::new()?;
    let mut app = worlds.app()?;
    app.play.set_paused(true);
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let key = |key, pressed, repeat| egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Default::default(),
    };
    frame_events(&ctx, &mut app, &mut worlds.assets, 0, true, false, vec![])?;
    let before = app.play.world_id();
    frame_events(
        &ctx,
        &mut app,
        &mut worlds.assets,
        1,
        true,
        false,
        vec![key(egui::Key::F5, true, false)],
    )?;
    assert!(
        app.checkpoints
            .as_ref()
            .unwrap()
            .load_checkpoint()?
            .is_some()
    );
    assert!(!app.checkpoints.as_ref().unwrap().is_persistent());
    assert_eq!(app.play.world_id(), before);
    let saved_notice = app.checkpoint_notice.as_ref().unwrap().1;
    frame_events(
        &ctx,
        &mut app,
        &mut worlds.assets,
        2,
        true,
        false,
        vec![key(egui::Key::F5, true, true)],
    )?;
    assert_eq!(app.checkpoint_notice.as_ref().unwrap().1, saved_notice);
    let restored = frame_events(
        &ctx,
        &mut app,
        &mut worlds.assets,
        3,
        true,
        false,
        vec![
            key(egui::Key::F5, false, false),
            key(egui::Key::F9, true, false),
        ],
    )?;
    assert_ne!(restored.world, before);
    assert!(app.play.is_paused());
    assert_eq!(app.play.steps(), 0);
    frame_events(
        &ctx,
        &mut app,
        &mut worlds.assets,
        4,
        true,
        false,
        vec![key(egui::Key::F9, true, true)],
    )?;
    assert_eq!(app.play.world_id(), restored.world);
    assert_eq!(
        app.play.hud_frame().unwrap().labels[0].text.as_ref(),
        "restored"
    );
    Ok(())
}
