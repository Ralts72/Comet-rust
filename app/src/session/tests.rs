use super::*;
mod animation;
mod checkpoint;
mod hud;
mod menu;
use comet_asset_data::product::{
    INDEX_VERSION, IndexEntry, PRODUCT_VERSION, Product, ProductFile, ProductIndex, SCRIPT_VERSION,
    ScriptData,
};
use comet_core::{AssetId, EntityId};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf};

struct Products(PathBuf);
impl Drop for Products {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Products {
    fn new(payloads: Vec<(AssetId, Product)>) -> Result<(Self, AssetStore)> {
        let root = Self(std::env::temp_dir().join(format!("comet-app-scenes-{}", AssetId::new())));
        let mut index = ProductIndex {
            version: INDEX_VERSION,
            products: BTreeMap::new(),
        };
        for (id, payload) in payloads {
            let product = ProductFile {
                version: PRODUCT_VERSION,
                id,
                source_digest: comet_core::digest(b"app scene fixture"),
                payload,
            };
            let bytes = serde_json::to_vec(&product)?;
            let digest = comet_core::digest(&bytes);
            let directory = root.0.join("products").join(id.to_string());
            fs::create_dir_all(&directory)?;
            fs::write(directory.join(format!("{digest}.json")), bytes)?;
            index.products.insert(
                id,
                IndexEntry {
                    digest,
                    kind: product.payload.kind(),
                    dependencies: product.payload.dependencies(),
                    editor: None,
                },
            );
        }
        fs::write(root.0.join("index.json"), serde_json::to_vec(&index)?)?;
        let assets = AssetStore::open(&root.0)?;
        Ok((root, assets))
    }
}

fn entity(name: &str, script: Option<AssetId>) -> Value {
    json!({
        "id":EntityId::new(), "name":name, "parent":null,
        "transform":{"translation":[0.,0.,0.],"rotation":[0.,0.,0.,1.],"scale":[1.,1.,1.]},
        "model":null,"links":{"target":null,"connections":[]},
        "script":script.map(|asset| json!({"asset":asset})),
    })
}
fn scene(entities: Vec<Value>) -> Result<Product> {
    Ok(Product::Scene(serde_json::from_value(json!({
        "version":comet_scene::prefab::CONTENT_VERSION,"base":{"version":comet_scene::scene::SCENE_VERSION,"entities":entities},"instances":[],
    }))?))
}
fn script(source: impl Into<String>) -> Product {
    Product::Script(ScriptData {
        version: SCRIPT_VERSION,
        source: source.into(),
    })
}
fn request_script(target: AssetId) -> Product {
    script(format!(
        r#"fn on_start(ctx) {{ ctx.log("start"); ctx.request_scene("{target}"); }}
           fn on_fixed_update(ctx, dt) {{ ctx.request_scene("{target}"); }}
           fn on_stop(ctx) {{ ctx.log("stop"); }}"#
    ))
}
fn app(
    assets: &mut AssetStore,
    entry: AssetId,
    allowed_scenes: Option<BTreeSet<AssetId>>,
) -> Result<App> {
    App::new(
        entry,
        assets,
        allowed_scenes,
        Ok(crate::storage::Store::memory(comet_core::GameId::new())),
    )
}
fn frame(app: &mut App, assets: &mut AssetStore, number: u64) -> Result<ViewRequest> {
    frame_with_activity(app, assets, number, true)
}
fn frame_with_activity(
    app: &mut App,
    assets: &mut AssetStore,
    number: u64,
    active: bool,
) -> Result<ViewRequest> {
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    frame_events(&ctx, app, assets, number, active, false, vec![])
}

fn frame_events(
    ctx: &egui::Context,
    app: &mut App,
    assets: &mut AssetStore,
    number: u64,
    active: bool,
    activity_changed: bool,
    events: Vec<egui::Event>,
) -> Result<ViewRequest> {
    Ok(frame_output(ctx, app, assets, number, active, activity_changed, events)?.0)
}

fn frame_output(
    ctx: &egui::Context,
    app: &mut App,
    assets: &mut AssetStore,
    number: u64,
    active: bool,
    activity_changed: bool,
    events: Vec<egui::Event>,
) -> Result<(ViewRequest, egui::FullOutput)> {
    frame_output_with_context(
        ctx,
        app,
        assets,
        FrameContext {
            gamepad: Default::default(),
            number,
            dt: 1. / 60.,
            active,
            activity_changed,
            target_generation: 1,
            size: [640, 480],
            smoke: false,
        },
        events,
    )
}
fn frame_output_with_context(
    ctx: &egui::Context,
    app: &mut App,
    assets: &mut AssetStore,
    input: FrameContext,
    events: Vec<egui::Event>,
) -> Result<(ViewRequest, egui::FullOutput)> {
    let mut view = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640., 480.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            view = Some(app.frame(ui, assets, &input));
        },
    );
    output.textures_delta.clear();
    Ok((view.context("app frame")??, output))
}

#[test]
fn an_inactive_host_defers_the_pending_request_until_resume() -> Result<()> {
    let a = AssetId::new();
    let b = AssetId::new();
    let program = AssetId::new();
    let (_products, mut assets) = Products::new(vec![
        (a, scene(vec![entity("first", Some(program))])?),
        (b, scene(vec![entity("resumed", None)])?),
        (program, request_script(b)),
    ])?;
    let mut app = app(&mut assets, a, None)?;
    let world = app.play.world_id();
    let suspended = frame_with_activity(&mut app, &mut assets, 0, false)?;
    assert_eq!(suspended.world, world);
    assert_eq!(app.scene, a);
    assert!(app.play.is_suspended());
    assert_eq!(app.play.steps(), 0);
    let resumed = frame(&mut app, &mut assets, 1)?;
    assert_ne!(resumed.world, world);
    assert_eq!(app.scene, b);
    assert!(!app.play.is_suspended());
    assert_eq!(app.play.steps(), 0);
    Ok(())
}

#[test]
fn package_scene_policy_rejects_incidental_dependencies_and_allows_later_retry() -> Result<()> {
    let a = AssetId::new();
    let b = AssetId::new();
    let program = AssetId::new();
    let next = entity("incidental scene", None);
    let mut first = entity("requester", Some(program));
    first["links"]["target"] = json!({"scope":"external","scene":b,"entity":next["id"]});
    let (_products, mut assets) = Products::new(vec![
        (a, scene(vec![first])?),
        (b, scene(vec![next])?),
        (program, request_script(b)),
    ])?;
    assert!(assets.dependency_closure(a)?.contains(&b));
    let mut app = app(&mut assets, a, Some(BTreeSet::from([a])))?;
    let before = app.play.shared_snapshot();
    let world = app.play.world_id();
    let rejected = frame(&mut app, &mut assets, 0)?;
    assert_eq!(rejected.world, world);
    assert_eq!(app.scene, a);
    assert!(std::sync::Arc::ptr_eq(&before, &app.play.shared_snapshot()));
    let error = app
        .play
        .script_diagnostics()
        .back()
        .context("denied request diagnostic")?;
    assert!(error.failed && error.message.contains("not included in this package"));
    assert!(app.play.take_scene_request().is_none());

    app.allowed_scenes.as_mut().unwrap().insert(b);
    let accepted = frame(&mut app, &mut assets, 1)?;
    assert_ne!(accepted.world, world);
    assert_eq!(app.scene, b);
    assert_eq!(accepted.snapshot.entities[0].name, "incidental scene");
    assert!(
        app.play
            .script_diagnostics()
            .iter()
            .any(|entry| entry.message == "stop")
    );
    Ok(())
}

#[test]
fn each_app_frame_accepts_only_one_scene_and_restart_uses_the_new_entry() -> Result<()> {
    let a = AssetId::new();
    let b = AssetId::new();
    let c = AssetId::new();
    let first_script = AssetId::new();
    let second_script = AssetId::new();
    let (_products, mut assets) = Products::new(vec![
        (a, scene(vec![entity("first", Some(first_script))])?),
        (b, scene(vec![entity("second", Some(second_script))])?),
        (c, scene(vec![entity("final", None)])?),
        (first_script, request_script(b)),
        (second_script, request_script(c)),
    ])?;
    let mut app = app(&mut assets, a, None)?;
    let old_world = app.play.world_id();
    let first_frame = frame(&mut app, &mut assets, 0)?;
    assert_eq!(app.scene, b);
    assert_ne!(first_frame.world, old_world);
    assert_eq!(first_frame.snapshot.entities[0].name, "second");
    let second_frame = frame(&mut app, &mut assets, 1)?;
    assert_eq!(app.scene, c);
    assert_ne!(second_frame.world, first_frame.world);
    assert_eq!(second_frame.snapshot.entities[0].name, "final");
    app.play.restart()?;
    assert_ne!(app.play.world_id(), second_frame.world);
    assert_eq!(app.play.shared_snapshot().entities[0].name, "final");
    assert_eq!(app.scene, c);
    assert_eq!(app.play.steps(), 0);
    assert_eq!(app.play.elapsed(), 0.);
    Ok(())
}

#[test]
fn invalid_target_or_script_keeps_the_running_world() -> Result<()> {
    for invalid in ["missing", "wrong kind", "script"] {
        let a = AssetId::new();
        let b = AssetId::new();
        let program = AssetId::new();
        let mut products = vec![
            (a, scene(vec![entity("running", Some(program))])?),
            (program, request_script(b)),
        ];
        match invalid {
            "wrong kind" => products.push((b, script("fn on_start(ctx) {}"))),
            "script" => {
                let broken = AssetId::new();
                products.push((b, scene(vec![entity("invalid program", Some(broken))])?));
                products.push((broken, script("fn on_start(ctx) { invalid syntax ??? }")));
            }
            _ => {}
        }
        let (_products, mut assets) = Products::new(products)?;
        let mut app = app(&mut assets, a, None)?;
        let world = app.play.world_id();
        let source = app.play.shared_snapshot();
        let view = frame(&mut app, &mut assets, 0)?;
        assert_eq!(view.world, world, "{invalid}");
        assert_eq!(app.scene, a);
        assert!(std::sync::Arc::ptr_eq(&source, &app.play.shared_snapshot()));
        assert!(app.play.script_diagnostics().back().unwrap().failed);
        assert!(app.play.take_scene_request().is_none());
    }
    Ok(())
}

#[test]
fn candidate_startup_draw_validation_precedes_world_replacement() -> Result<()> {
    let a = AssetId::new();
    let b = AssetId::new();
    let first_script = AssetId::new();
    let second_script = AssetId::new();
    let mesh = AssetId::new();
    let material = AssetId::new();
    let mut visual = entity("unreliable GPU transform", Some(second_script));
    visual["model"] = json!({"mesh":mesh,"surface":{"material":material,"roughness":0.5}});
    let mut surface: comet_asset_data::product::MaterialData = serde_json::from_slice(
        include_bytes!("../../../tests/fixtures/prefab/assets/material.material"),
    )?;
    surface.base_color_texture = None;
    let (_products, mut assets) = Products::new(vec![
        (a, scene(vec![entity("old world", Some(first_script))])?),
        (b, scene(vec![visual])?),
        (first_script, request_script(b)),
        (
            second_script,
            script("fn on_start(ctx) { ctx.set_scale(ctx.entity(), 1e-40, 1e-40, 1e-40); }"),
        ),
        (
            mesh,
            Product::Mesh(serde_json::from_slice(include_bytes!(
                "../../../tests/fixtures/prefab/assets/mesh.mesh"
            ))?),
        ),
        (material, Product::Material(surface)),
    ])?;
    let mut app = app(&mut assets, a, None)?;
    let world = app.play.world_id();
    let source = app.play.shared_snapshot();
    let view = frame(&mut app, &mut assets, 0)?;
    assert_eq!(view.world, world);
    assert_eq!(app.scene, a);
    assert!(std::sync::Arc::ptr_eq(&source, &app.play.shared_snapshot()));
    let error = app
        .play
        .script_diagnostics()
        .back()
        .context("draw preflight diagnostic")?;
    assert!(error.failed && error.message.contains("render transform"));
    Ok(())
}
