use crate::timing::FrameTiming;
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_core::{EntityId, WorldId};
use comet_render::{Camera, FrameFeedback, FramePacket, RenderWorker, Surface, UiFrame, ViewToken};
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
    pub smoke: bool,
    pub headless: bool,
    pub capture: Option<PathBuf>,
    pub report: Option<PathBuf>,
}
impl HostOptions {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.into(),
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
    pub snapshot: Arc<SceneSnapshot>,
    pub viewport: egui::Rect,
    pub camera: Camera,
    pub selected: Option<EntityId>,
    pub pick: Option<[f32; 2]>,
    pub shadows: bool,
}
pub trait Application {
    fn frame(
        &mut self,
        ui: &mut egui::Ui,
        assets: &mut AssetStore,
        context: &FrameContext,
    ) -> Result<ViewRequest>;
    fn feedback(&mut self, _feedback: &FrameFeedback) {}
    fn report(&mut self) -> serde_json::Value {
        serde_json::json!({})
    }
}
struct Host<A: Application> {
    worker: Option<RenderWorker>,
    window: Option<Arc<Window>>,
    state: Option<egui_winit::State>,
    options: HostOptions,
    app: A,
    assets: AssetStore,
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
    } else {
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut host)?;
    }
    if let Some(worker) = host.worker.as_mut() {
        worker.close()?;
    }
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
                &serde_json::json!({"frames":host.feedback,"submitted_frames":host.number,"application":host.app.report(),"headless":headless}),
            )?,
        )?;
    }
    Ok(())
}
impl<A: Application> Host<A> {
    fn setup(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title(&self.options.title)
                    .with_inner_size(LogicalSize::new(1280., 800.))
                    .with_min_inner_size(LogicalSize::new(1000., 650.)),
            )?,
        );
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
        let mut request = None;
        let mut output = self.ctx.run_ui(raw, |ui| {
            request = Some(self.app.frame(ui, &mut self.assets, &input));
        });
        if let (Some(window), Some(state)) = (&self.window, &mut self.state) {
            state.handle_platform_output(window, output.platform_output.clone());
        }
        let view = request.context("application did not produce a view")??;
        let ppp = output.pixels_per_point;
        let rect = view.viewport * ppp;
        let x = rect.min.x.clamp(0., size[0] as f32) as u32;
        let y = rect.min.y.clamp(0., size[1] as f32) as u32;
        let w = (rect.width().max(0.) as u32).min(size[0] - x);
        let h = (rect.height().max(0.) as u32).min(size[1] - y);
        let token = ViewToken {
            world: view.world,
            view: 1,
            target_generation: self.generation,
        };
        self.token = Some(token);
        let done = self.number == 7 && (self.options.smoke || self.options.headless);
        let frame = FramePacket {
            number: self.number,
            token,
            size,
            viewport: [x, y, w, h],
            camera: view.camera,
            draws: crate::extract(&view.snapshot, &mut self.assets, view.selected)?,
            ui: UiFrame {
                primitives: self.ctx.tessellate(std::mem::take(&mut output.shapes), ppp),
                textures: std::mem::take(&mut output.textures_delta),
                pixels_per_point: ppp,
            },
            pick: view.pick,
            capture: if done {
                self.options.capture.clone()
            } else {
                None
            },
            shadows: view.shadows,
        };
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
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let (Some(window), Some(state)) = (&self.window, &mut self.state) {
            let _ = state.on_window_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.closing = true;
                event_loop.exit();
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
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
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
