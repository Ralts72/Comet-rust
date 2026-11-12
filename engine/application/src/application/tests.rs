use super::*;
use comet_asset_data::product::{INDEX_VERSION, ProductIndex};
use comet_core::AssetId;
use comet_scene::{EntityData, Model, Surface};
use std::{collections::BTreeMap, fs};

struct Products(PathBuf);
impl Products {
    fn empty() -> Result<(Self, AssetStore)> {
        let path = std::env::temp_dir().join(format!("comet-application-view-{}", AssetId::new()));
        fs::create_dir(&path)?;
        let products = Self(path);
        fs::write(
            products.0.join("index.json"),
            serde_json::to_vec(&ProductIndex {
                version: INDEX_VERSION,
                products: BTreeMap::new(),
            })?,
        )?;
        let store = AssetStore::open(&products.0)?;
        Ok((products, store))
    }
}
impl Drop for Products {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn empty_view(world: WorldId) -> ViewRequest {
    ViewRequest {
        world,
        hud: None,
        snapshot: Arc::new(SceneSnapshot::default()),
        viewport: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640., 480.)),
        camera: Camera::default(),
        selected: vec![],
        pick: None,
        shadows: true,
    }
}

fn missing_product_view(world: WorldId) -> (ViewRequest, AssetId) {
    let mesh = AssetId::new();
    let mut entity = EntityData::new("missing product");
    entity.model = Some(Model {
        mesh,
        surface: Surface {
            material: AssetId::new(),
            roughness: 0.5,
        },
    });
    let mut view = empty_view(world);
    view.selected.push(entity.id);
    view.pick = Some(PickRequest::Point([0.5, 0.5]));
    view.snapshot = Arc::new(SceneSnapshot {
        entities: vec![entity],
        ..Default::default()
    });
    (view, mesh)
}

struct Strict {
    request: Option<Result<ViewRequest>>,
    frames: usize,
    close: bool,
    text: &'static str,
}
impl Strict {
    fn new(request: Result<ViewRequest>) -> Self {
        Self {
            request: Some(request),
            frames: 0,
            close: false,
            text: "A",
        }
    }
}
impl Application for Strict {
    fn close_ready(&self) -> bool {
        self.close
    }
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        _: &mut AssetStore,
        _: &FrameContext,
    ) -> Result<ViewRequest> {
        self.frames += 1;
        ui.label(self.text);
        self.request.take().context("frame callback repeated")?
    }
}

struct Recovering {
    inner: Strict,
    fallback: Option<Result<ViewRequest>>,
    errors: Vec<String>,
}
impl Recovering {
    fn new(request: Result<ViewRequest>, fallback: Result<ViewRequest>) -> Self {
        Self {
            inner: Strict::new(request),
            fallback: Some(fallback),
            errors: vec![],
        }
    }
}
impl Application for Recovering {
    fn close_ready(&self) -> bool {
        self.inner.close_ready()
    }
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        input: &FrameContext,
    ) -> Result<ViewRequest> {
        self.inner.frame(ui, assets, input)
    }
    fn view_error(&mut self, error: anyhow::Error) -> Result<ViewRequest> {
        self.errors.push(format!("{error:#}"));
        self.fallback.take().context("recovery callback repeated")?
    }
}

fn host<A: Application>(assets: AssetStore, app: A) -> Host<A> {
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    Host {
        worker: None,
        window: None,
        state: None,
        clipboard: None,
        options: HostOptions::new("application view test"),
        app,
        assets,
        extractor: Default::default(),
        audio: Default::default(),
        final_audio: None,
        ctx,
        number: 0,
        generation: 1,
        last_size: [640, 480],
        timing: FrameTiming::default(),
        token: None,
        feedback: vec![],
        error: None,
        closing: false,
    }
}

fn prepare(host: &mut Host<impl Application>) -> Result<Option<FramePacket>> {
    host.prepare_frame(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640., 480.),
            )),
            ..Default::default()
        },
        &FrameContext {
            number: host.number,
            dt: 0.,
            active: true,
            activity_changed: false,
            target_generation: 17,
            size: [640, 480],
            smoke: false,
        },
    )
}

fn frame_error(result: Result<Option<FramePacket>>) -> anyhow::Error {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected a fatal content error"),
    }
}

#[test]
fn default_callback_errors_stay_fatal_and_retire_first_font_updates() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let error = anyhow::anyhow!("original callback error").context("view callback");
    let mut host = host(assets, Strict::new(Err(error)));
    let error = format!("{:#}", frame_error(prepare(&mut host)));
    assert_eq!(error, "view callback: original callback error");
    assert_eq!(host.app.frames, 1);
    assert!(host.token.is_none());
    Ok(())
}

#[test]
fn default_extraction_errors_stay_fatal_without_mutating_the_source() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let (view, mesh) = missing_product_view(WorldId::new());
    let source = view.snapshot.clone();
    let before = source.as_ref().clone();
    let mut host = host(assets, Strict::new(Ok(view)));
    let error = format!("{:#}", frame_error(prepare(&mut host)));
    assert!(error.contains(&format!("asset not indexed {mesh}")));
    assert_eq!(source.as_ref(), &before);
    assert_eq!(host.app.frames, 1);
    assert!(host.token.is_none());
    Ok(())
}

#[test]
fn missing_products_recover_once_and_deliver_ui_in_the_replacement_world() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let (view, mesh) = missing_product_view(WorldId::new());
    let replacement = WorldId::new();
    let mut host = host(
        assets,
        Recovering::new(Ok(view), Ok(empty_view(replacement))),
    );
    let frame = prepare(&mut host)?.context("recovered frame")?;
    assert_eq!(host.app.inner.frames, 1);
    assert_eq!(host.app.errors.len(), 1);
    assert!(host.app.errors[0].contains(&mesh.to_string()));
    assert!(frame.draws.is_empty());
    assert!(frame.pick.is_none());
    assert_eq!(frame.token.world, replacement);
    assert_eq!(frame.token.target_generation, 17);
    assert_eq!(host.token, Some(frame.token));
    assert!(!frame.ui.textures.is_empty());
    assert!(!frame.ui.primitives.is_empty());
    frame.validate()?;
    Ok(())
}

#[test]
fn callback_errors_recover_without_replaying_frame_side_effects() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let app = Recovering::new(
        Err(anyhow::anyhow!("invalid application content")),
        Ok(empty_view(WorldId::new())),
    );
    let mut host = host(assets, app);
    let frame = prepare(&mut host)?.context("recovered frame")?;
    assert_eq!(host.app.inner.frames, 1);
    assert_eq!(host.app.errors, ["invalid application content"]);
    assert!(frame.draws.is_empty());
    assert!(!frame.ui.textures.is_empty());
    Ok(())
}

#[test]
fn an_invalid_fallback_is_fatal_after_one_recovery_attempt() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let (view, first_mesh) = missing_product_view(WorldId::new());
    let (fallback, fallback_mesh) = missing_product_view(WorldId::new());
    let mut host = host(assets, Recovering::new(Ok(view), Ok(fallback)));
    let error = format!("{:#}", frame_error(prepare(&mut host)));
    assert!(error.contains(&fallback_mesh.to_string()));
    assert_eq!(host.app.inner.frames, 1);
    assert_eq!(host.app.errors.len(), 1);
    assert!(host.app.errors[0].contains(&first_mesh.to_string()));
    assert!(host.token.is_none());
    Ok(())
}

#[test]
fn successful_content_never_calls_recovery() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let world = WorldId::new();
    let mut host = host(
        assets,
        Recovering::new(
            Ok(empty_view(world)),
            Err(anyhow::anyhow!("unused fallback")),
        ),
    );
    let frame = prepare(&mut host)?.context("normal frame")?;
    assert_eq!(frame.token.world, world);
    assert_eq!(host.app.inner.frames, 1);
    assert!(host.app.errors.is_empty());
    assert!(!frame.ui.textures.is_empty());
    Ok(())
}

#[test]
fn reused_draws_keep_the_current_camera_world_viewport_and_pick() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let mut first_view = empty_view(WorldId::new());
    let snapshot = first_view.snapshot.clone();
    first_view.pick = Some(PickRequest::Point([0.5, 0.5]));
    let mut host = host(assets, Strict::new(Ok(first_view)));
    let first = prepare(&mut host)?.context("initial frame")?;

    let world = WorldId::new();
    let mut next_view = empty_view(world);
    next_view.snapshot = snapshot;
    next_view.camera.eye.x += 2.;
    let camera = next_view.camera;
    next_view.viewport = egui::Rect::from_min_size(egui::pos2(10., 20.), egui::vec2(300., 200.));
    next_view.pick = Some(PickRequest::Point([0.25, 0.75]));
    next_view.shadows = false;
    host.app.request = Some(Ok(next_view));
    let second = host
        .prepare_frame(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                ..Default::default()
            },
            &FrameContext {
                number: 2,
                dt: 0.,
                active: true,
                activity_changed: false,
                target_generation: 23,
                size: [800, 600],
                smoke: false,
            },
        )?
        .context("updated view frame")?;
    assert!(Arc::ptr_eq(&first.draws, &second.draws));
    assert_eq!(host.app.frames, 2);
    assert_ne!(first.token.world, second.token.world);
    assert_eq!(second.token.world, world);
    assert_eq!(second.token.target_generation, 23);
    assert_eq!(host.token, Some(second.token));
    assert_eq!(second.number, 2);
    assert_eq!(second.size, [800, 600]);
    assert_eq!(second.viewport, [10, 20, 300, 200]);
    assert_eq!(second.camera.eye, camera.eye);
    assert_eq!(second.camera.target, camera.target);
    assert_eq!(second.pick, Some(PickRequest::Point([0.25, 0.75])));
    assert!(!second.shadows);
    assert_eq!(first.pick, Some(PickRequest::Point([0.5, 0.5])));
    second.validate()?;
    Ok(())
}

#[test]
fn approved_close_retires_font_updates_without_preparing_a_frame() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let mut app = Strict::new(Ok(empty_view(WorldId::new())));
    app.close = true;
    let mut host = host(assets, app);
    assert!(prepare(&mut host)?.is_none());
    assert!(host.closing);
    assert!(host.token.is_none());
    assert_eq!(host.app.frames, 1);
    Ok(())
}

#[test]
fn later_glyph_updates_are_preserved_on_recovery_and_retired_on_close_or_error() -> Result<()> {
    for outcome in ["recover", "close", "fatal"] {
        let (_products, assets) = Products::empty()?;
        let world = WorldId::new();
        let mut host = host(
            assets,
            Recovering::new(Ok(empty_view(world)), Ok(empty_view(world))),
        );
        let first = prepare(&mut host)?.context("initial font frame")?;
        assert!(!first.ui.textures.is_empty());
        drop(first);
        host.app.inner.text = "λЖΩ∑";
        host.app.inner.request = Some(Err(anyhow::anyhow!("later callback error")));
        match outcome {
            "recover" => {
                let frame = prepare(&mut host)?.context("new glyph frame")?;
                assert!(
                    frame
                        .ui
                        .textures
                        .set
                        .values()
                        .flatten()
                        .any(|d| d.pos.is_some())
                );
                assert_eq!(host.app.errors.len(), 1);
            }
            "close" => {
                host.app.inner.close = true;
                assert!(prepare(&mut host)?.is_none());
            }
            "fatal" => {
                host.app.fallback = Some(Err(anyhow::anyhow!("recovery refused")));
                assert!(
                    frame_error(prepare(&mut host))
                        .to_string()
                        .contains("recovery refused")
                );
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[test]
fn audio_shutdown_preserves_the_first_pre_shutdown_observation_and_retires_pending_playback()
-> Result<()> {
    use comet_asset_data::{AudioData, audio::AUDIO_VERSION};
    use comet_audio::{AudioFrame, AudioState, AudioVoice};
    use comet_core::EntityId;

    struct PausedAudio {
        inner: Strict,
        audio: AudioFrame,
    }
    impl Application for PausedAudio {
        fn frame(
            &mut self,
            ui: &mut egui::Ui,
            assets: &mut AssetStore,
            input: &FrameContext,
        ) -> Result<ViewRequest> {
            self.inner.frame(ui, assets, input)
        }
        fn audio_frame(&mut self) -> Result<Option<AudioFrame>> {
            Ok(Some(self.audio.clone()))
        }
    }

    let (_products, assets) = Products::empty()?;
    let world = WorldId::new();
    let mut host = host(
        assets,
        PausedAudio {
            inner: Strict::new(Ok(empty_view(world))),
            audio: AudioFrame {
                world,
                paused: true,
                listener: None,
                voices: Arc::from([AudioVoice {
                    entity: EntityId::new(),
                    generation: 1,
                    clip: AssetId::new(),
                    playing: true,
                    looping: false,
                    gain: 1.,
                    spatial: None,
                    data: Arc::new(AudioData {
                        version: AUDIO_VERSION,
                        sample_rate: 48_000,
                        channels: 1,
                        samples: Arc::from([0, 8192, -8192, 0]),
                    }),
                }]),
            },
        },
    );
    let packet = prepare(&mut host)?.context("accepted view and paused audio")?;
    assert_eq!(packet.token.world, world);
    assert_eq!(host.app.inner.frames, 1);
    assert!(host.final_audio.is_none());
    assert_eq!(host.audio.report().state, AudioState::Idle);
    assert_eq!(host.audio.report().pending_voices, 1);
    assert_eq!(host.audio.report().generations_started, 0);

    host.shutdown_audio();
    let final_audio = host
        .final_audio
        .as_ref()
        .context("pre-shutdown audio snapshot")?;
    assert_eq!(final_audio.world, Some(world));
    assert_eq!(final_audio.pending_voices, 1);
    assert_eq!(final_audio.voices[0].generation, 1);
    let saved = serde_json::to_value(final_audio)?;
    let stopped = host.audio.report();
    assert_eq!(stopped.pending_voices, 0);
    assert_eq!(stopped.active_voices, 0);
    assert_eq!(stopped.generations_started, 0);
    assert!(stopped.world.is_none());
    host.shutdown_audio();
    assert_eq!(
        serde_json::to_value(host.final_audio.as_ref().unwrap())?,
        saved
    );
    assert_eq!(host.audio.report().pending_voices, 0);
    Ok(())
}

fn hud(world: WorldId, text: &str) -> comet_core::HudFrame {
    comet_core::HudFrame {
        world,
        labels: Arc::from([comet_core::HudLabel {
            owner: EntityId::new(),
            text: Arc::from(text),
        }]),
    }
}

#[test]
fn accepted_hud_uses_the_recovered_world_and_uploads_new_glyphs_in_the_same_pass() -> Result<()> {
    let (_products, assets) = Products::empty()?;
    let original = WorldId::new();
    let replacement = WorldId::new();
    let mut fallback = empty_view(replacement);
    fallback.hud = Some(hud(replacement, "任务：收集目标并继续"));
    let mut host = host(
        assets,
        Recovering::new(Ok(empty_view(original)), Ok(fallback)),
    );
    crate::configure_ui(&host.ctx);
    drop(prepare(&mut host)?.context("warm UI frame")?);
    let mut rejected = empty_view(original);
    rejected.hud = Some(hud(replacement, "旧世界内容"));
    host.app.inner.request = Some(Ok(rejected));
    host.number += 1;
    let frame = prepare(&mut host)?.context("recovered HUD frame")?;
    assert_eq!(host.app.inner.frames, 2);
    assert_eq!(host.app.errors.len(), 1);
    assert!(host.app.errors[0].contains("HUD"));
    assert_eq!(frame.token.world, replacement);
    assert!(
        !frame.ui.textures.is_empty(),
        "HUD glyphs must not wait for another pass"
    );
    assert!(!frame.ui.primitives.is_empty());
    frame.validate()?;
    Ok(())
}

#[test]
fn invalid_hud_in_a_fallback_is_fatal_after_one_recovery_without_replaying_the_application()
-> Result<()> {
    let (_products, assets) = Products::empty()?;
    let world = WorldId::new();
    let mut original = empty_view(world);
    original.hud = Some(hud(WorldId::new(), "wrong world"));
    let mut fallback = empty_view(world);
    fallback.hud = Some(hud(world, &"x".repeat(comet_core::MAX_HUD_LABEL_BYTES + 1)));
    let mut host = host(assets, Recovering::new(Ok(original), Ok(fallback)));
    let error = frame_error(prepare(&mut host));
    assert!(format!("{error:#}").contains("HUD"));
    assert_eq!(host.app.inner.frames, 1);
    assert_eq!(host.app.errors.len(), 1);
    assert!(host.app.fallback.is_none());
    assert!(host.token.is_none());
    Ok(())
}
