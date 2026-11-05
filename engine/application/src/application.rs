use crate::timing::FrameTiming;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_core::{EntityId, WorldId};
use comet_render::{
    Camera, Draw, FrameFeedback, FramePacket, PickRequest, RenderWorker, Surface, UiFrame,
    ViewToken,
};
use comet_scene::SceneSnapshot;
use std::{path::PathBuf, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

pub struct HostOptions {
    pub title: String,
    pub icon: Option<std::borrow::Cow<'static, [u8]>>,
    pub smoke: bool,
    pub headless: bool,
    pub capture: Option<PathBuf>,
    pub report: Option<PathBuf>,
}
impl HostOptions {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.into(),
            icon: None,
            smoke: false,
            headless: false,
            capture: None,
            report: None,
        }
    }
}
pub struct FrameContext {
    pub number: u64,
    pub dt: f64,
    pub active: bool,
    pub activity_changed: bool,
    pub target_generation: u64,
    pub size: [u32; 2],
    pub smoke: bool,
}
pub struct ViewRequest {
    pub world: WorldId,
    pub hud: Option<comet_core::HudFrame>,
    pub snapshot: Arc<SceneSnapshot>,
    pub viewport: egui::Rect,
    pub camera: Camera,
    /// Selected objects; extraction highlights their rendered descendants as well.
    pub selected: Vec<EntityId>,
    pub pick: Option<PickRequest>,
    pub shadows: bool,
}
pub trait Application {
    /// Return false to keep the host alive while the application resolves a close request.
    fn request_close(&mut self) -> bool {
        true
    }
    fn close_ready(&self) -> bool {
        false
    }
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        context: &FrameContext,
    ) -> Result<ViewRequest>;
    /// Recover application content errors; renderer and worker failures remain fatal.
    fn view_error(&mut self, error: anyhow::Error) -> Result<ViewRequest> {
        Err(error)
    }
    fn feedback(&mut self, _feedback: &FrameFeedback) {}
    fn audio_frame(&mut self) -> Result<Option<comet_audio::AudioFrame>> {
        Ok(None)
    }
    /// Accepted host output volume, independent of runtime pause and individual sound sources.
    fn audio_master_gain(&self) -> f32 {
        1.
    }
    fn audio_error(&mut self, error: &str) {
        eprintln!("音频输出不可用：{error}");
    }
    fn report(&mut self) -> serde_json::Value {
        serde_json::json!({})
    }
}
fn prepare_view(
    app: &mut impl Application,
    assets: &mut AssetStore,
    request: Result<ViewRequest>,
    extractor: &mut crate::extract::SceneExtractor,
) -> Result<(ViewRequest, Arc<[Draw]>)> {
    let mut extract = |view: ViewRequest| -> Result<_> {
        if let Some(hud) = &view.hud {
            ensure!(
                hud.world == view.world,
                "HUD does not belong to the accepted view world"
            );
            hud.validate()?;
        }
        let draws = extractor.extract(&view.snapshot, assets, &view.selected)?;
        Ok((view, draws))
    };
    let result = request.and_then(&mut extract);
    match result {
        Ok(prepared) => Ok(prepared),
        Err(error) => extract(app.view_error(error)?),
    }
}
struct Host<A: Application> {
    worker: Option<RenderWorker>,
    window: Option<Arc<Window>>,
    state: Option<egui_winit::State>,
    options: HostOptions,
    app: A,
    assets: AssetStore,
    extractor: crate::extract::SceneExtractor,
    audio: crate::audio::AudioHost,
    final_audio: Option<comet_audio::AudioReport>,
    ctx: egui::Context,
    number: u64,
    generation: u64,
    last_size: [u32; 2],
    timing: FrameTiming,
    token: Option<ViewToken>,
    feedback: Vec<FrameFeedback>,
    error: Option<String>,
    closing: bool,
}
pub fn run<A: Application>(options: HostOptions, assets: AssetStore, app: A) -> Result<()> {
    let headless = options.headless;
    let ctx = egui::Context::default();
    crate::configure_ui(&ctx);
    // Application callbacks include commands and one runtime update, so execute exactly once.
    // Layout settles on the next host frame rather than replaying side effects in an egui pass.
    ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
    let mut host = Host {
        worker: None,
        window: None,
        state: None,
        options,
        app,
        assets,
        extractor: Default::default(),
        audio: Default::default(),
        final_audio: None,
        ctx,
        number: 0,
        generation: 1,
        last_size: [1280, 800],
        timing: FrameTiming::default(),
        token: None,
        feedback: vec![],
        error: None,
        closing: false,
    };
    if headless {
        host.worker = Some(RenderWorker::spawn(None)?);
        for _ in 0..8 {
            host.draw()?;
        }
        if host.options.smoke {
            host.audio
                .wait_for_progress(std::time::Duration::from_millis(500));
        }
    } else {
        #[cfg(target_os = "macos")]
        crate::macos::prepare()?;
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut host)?;
    }
    if let Some(worker) = host.worker.as_mut() {
        worker.close()?;
    }
    host.shutdown_audio();
    let audio = host
        .final_audio
        .take()
        .unwrap_or_else(|| host.audio.report());
    let audio_stopped = host.audio.report();
    if let Some(error) = host.error {
        anyhow::bail!(error);
    }
    if let Some(report) = host.options.report {
        if let Some(parent) = report.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            report,
            serde_json::to_vec_pretty(
                &serde_json::json!({"frames":host.feedback,"submitted_frames":host.number,"application":host.app.report(),"audio":audio,"audio_shutdown":audio_stopped.active_voices == 0 && audio_stopped.pending_voices == 0,"headless":headless}),
            )?,
        )?;
    }
    Ok(())
}
impl<A: Application> Host<A> {
    fn shutdown_audio(&mut self) {
        if self.final_audio.is_none() {
            self.final_audio = Some(self.audio.report());
        }
        self.audio.shutdown();
    }
    fn setup(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let icon = self
            .options
            .icon
            .as_deref()
            .map(crate::icon::decode)
            .transpose()?;
        let attributes = Window::default_attributes()
            .with_title(&self.options.title)
            .with_window_icon(icon.clone())
            .with_inner_size(LogicalSize::new(1280., 800.))
            .with_min_inner_size(LogicalSize::new(1000., 650.));
        #[cfg(windows)]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_taskbar_icon(icon)
        };
        let window = Arc::new(event_loop.create_window(attributes)?);
        window.set_ime_allowed(true);
        self.worker = Some(RenderWorker::spawn(Some(Surface::new(window.clone())?))?);
        self.state = Some(egui_winit::State::new(
            self.ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            None,
            Some(16384),
        ));
        window.request_redraw();
        self.window = Some(window);
        Ok(())
    }
    fn accept(&mut self, feedback: FrameFeedback) {
        if Some(feedback.token) == self.token {
            self.app.feedback(&feedback);
        }
        if self.feedback.len() == 120 {
            self.feedback.remove(0);
        }
        self.feedback.push(feedback);
    }
    fn draw(&mut self) -> Result<bool> {
        if self.closing {
            return Ok(true);
        }
        let worker = self.worker.as_ref().context("render worker unavailable")?;
        for feedback in worker.drain()? {
            self.accept(feedback);
        }
        let actual_size = self
            .window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                [s.width, s.height]
            })
            .unwrap_or([1280, 800]);
        let size = if self.options.smoke && self.number == 2 {
            [0, 0]
        } else {
            actual_size
        };
        if size != self.last_size {
            self.generation += 1;
            self.last_size = size;
        }
        let host_dt = self.timing.sample(Instant::now());
        let active = self.timing.active();
        let activity_changed = self.timing.take_activity_change();
        // Eight very fast smoke frames must still exercise real simulation ticks.
        let dt = if self.options.smoke {
            if active && !activity_changed {
                1. / 60.
            } else {
                0.
            }
        } else {
            host_dt
        };
        let input = FrameContext {
            number: self.number,
            dt,
            active,
            activity_changed,
            target_generation: self.generation,
            size,
            smoke: self.options.smoke,
        };
        let raw = if let (Some(window), Some(state)) = (&self.window, &mut self.state) {
            state.take_egui_input(window)
        } else {
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280., 800.),
                )),
                ..Default::default()
            }
        };
        let Some(frame) = self.prepare_frame(raw, &input)? else {
            return Ok(true);
        };
        let done = self.number == 7 && (self.options.smoke || self.options.headless);
        self.worker.as_ref().unwrap().submit(frame)?;
        if done {
            let feedback = self.worker.as_ref().unwrap().wait(self.number)?;
            self.accept(feedback);
        }
        self.number += 1;
        self.closing = done;
        if self.options.smoke
            && self.number == 4
            && let Some(window) = &self.window
        {
            let _ = window.request_inner_size(LogicalSize::new(1100., 720.));
        }
        Ok(done)
    }
    fn prepare_frame(
        &mut self,
        raw: egui::RawInput,
        input: &FrameContext,
    ) -> Result<Option<FramePacket>> {
        let mut prepared = None;
        let mut output = self.ctx.run_ui(raw, |ui| {
            let request = self.app.frame(ui, &mut self.assets, input);
            prepared = Some(if self.app.close_ready() {
                Ok(None)
            } else {
                prepare_view(
                    &mut self.app,
                    &mut self.assets,
                    request,
                    &mut self.extractor,
                )
                .map(|(view, draws)| {
                    if let Some(hud) = &view.hud {
                        crate::hud::paint(ui, view.viewport, hud);
                    }
                    Some((view, draws))
                })
            });
        });
        // UiFrame acknowledges unsubmitted updates on fatal errors or approved close.
        let mut ui_frame = UiFrame {
            textures: std::mem::take(&mut output.textures_delta),
            pixels_per_point: output.pixels_per_point,
            primitives: vec![],
        };
        if let (Some(window), Some(state)) = (&self.window, &mut self.state) {
            state.handle_platform_output(window, output.platform_output.clone());
        }
        let Some((view, draws)) = prepared.context("application did not prepare a view")?? else {
            self.closing = true;
            return Ok(None);
        };
        self.audio
            .submit(&mut self.app, view.world, view.camera, input.active);
        let ppp = output.pixels_per_point;
        let size = input.size;
        let rect = view.viewport * ppp;
        let x = rect.min.x.clamp(0., size[0] as f32) as u32;
        let y = rect.min.y.clamp(0., size[1] as f32) as u32;
        let w = (rect.width().max(0.) as u32).min(size[0] - x);
        let h = (rect.height().max(0.) as u32).min(size[1] - y);
        let token = ViewToken {
            world: view.world,
            view: 1,
            target_generation: input.target_generation,
        };
        self.token = Some(token);
        let done = input.number == 7 && (self.options.smoke || self.options.headless);
        ui_frame.primitives = self.ctx.tessellate(std::mem::take(&mut output.shapes), ppp);
        Ok(Some(FramePacket {
            number: input.number,
            token,
            size,
            viewport: [x, y, w, h],
            camera: view.camera,
            draws,
            ui: ui_frame,
            pick: view.pick,
            capture: if done {
                self.options.capture.clone()
            } else {
                None
            },
            shadows: view.shadows,
        }))
    }
}
impl<A: Application> ApplicationHandler for Host<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.timing.suspended(false);
        if self.window.is_none()
            && let Err(error) = self.setup(event_loop)
        {
            self.error = Some(format!("{error:#}"));
            event_loop.exit();
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.timing.suspended(true);
        self.audio.pause(&mut self.app);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let (Some(window), Some(state)) = (&self.window, &mut self.state) {
            let _ = state.on_window_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.app.request_close() {
                    self.closing = true;
                    event_loop.exit();
                } else if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => match self.draw() {
                Ok(true) => event_loop.exit(),
                Ok(false) => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Err(error) => {
                    self.error = Some(format!("{error:#}"));
                    event_loop.exit();
                }
            },
            WindowEvent::Focused(focused) => self.timing.focused(focused),
            WindowEvent::Occluded(occluded) => self.timing.occluded(occluded),
            WindowEvent::Resized(size) => {
                self.timing.minimized(size.width == 0 || size.height == 0);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
        if !self.timing.active() {
            self.audio.pause(&mut self.app);
        }
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.shutdown_audio();
        if let Some(worker) = &mut self.worker
            && let Err(error) = worker.close()
        {
            self.error.get_or_insert_with(|| format!("{error:#}"));
        }
    }
}

/// Shared orbit math; the application decides when the viewport owns these gestures.
pub fn orbit(camera: &mut Camera, response: &egui::Response) {
    let mut offset = camera.eye - camera.target;
    if response.dragged_by(egui::PointerButton::Secondary) {
        let delta = response.ctx.input(|i| i.pointer.delta());
        offset = comet_core::math::Quat::from_rotation_y(-delta.x * 0.006) * offset;
        let right = offset.cross(comet_core::math::Vec3::Y).normalize_or_zero();
        let candidate = comet_core::math::Quat::from_axis_angle(right, delta.y * 0.006) * offset;
        if candidate.normalize().dot(comet_core::math::Vec3::Y).abs() < 0.96 {
            offset = candidate;
        }
    }
    if response.hovered() {
        let scroll = response.ctx.input(|i| i.smooth_scroll_delta.y);
        offset *= (-scroll * 0.002).exp().clamp(0.8, 1.2);
    }
    let distance = offset.length().clamp(1., 80.);
    camera.eye = camera.target + offset.normalize() * distance;
}
pub fn parse_common(
    options: &mut HostOptions,
    argument: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<bool> {
    match argument {
        "--smoke" => options.smoke = true,
        "--headless" => options.headless = true,
        "--capture" => {
            options.capture = Some(args.next().context("--capture requires a path")?.into())
        }
        "--report" => {
            options.report = Some(args.next().context("--report requires a path")?.into())
        }
        _ => return Ok(false),
    }
    ensure!(!options.title.is_empty(), "window title is empty");
    Ok(true)
}

#[cfg(test)]
mod tests;
