use super::*;
use crate::{FrameContext, ViewRequest};
use anyhow::Result;
use comet_asset::AssetStore;
use comet_asset_data::{AudioData, audio::AUDIO_VERSION};
use comet_audio::{AudioState, AudioVoice};
use comet_core::{AssetId, EntityId, WorldId};
use std::sync::Arc;

struct App {
    frame: Option<AudioFrame>,
    audio_reads: usize,
    errors: Vec<String>,
    failure: Option<String>,
    master_gain: f32,
}
impl App {
    fn new(frame: AudioFrame) -> Self {
        Self {
            frame: Some(frame),
            audio_reads: 0,
            errors: Vec::new(),
            failure: None,
            master_gain: 1.,
        }
    }
    fn voice(&mut self) -> &mut AudioVoice {
        &mut Arc::make_mut(&mut self.frame.as_mut().unwrap().voices)[0]
    }
}
impl Application for App {
    fn frame(
        &mut self,
        _: &mut egui::Ui,
        _: &mut AssetStore,
        _: &FrameContext,
    ) -> Result<ViewRequest> {
        panic!("audio events must not execute application updates");
    }
    fn audio_frame(&mut self) -> Result<Option<AudioFrame>> {
        self.audio_reads += 1;
        if let Some(error) = &self.failure {
            anyhow::bail!("{error}");
        }
        Ok(self.frame.clone())
    }
    fn audio_master_gain(&self) -> f32 {
        self.master_gain
    }
    fn audio_error(&mut self, error: &str) {
        self.errors.push(error.into());
    }
}

#[test]
fn host_volume_changes_do_not_initialize_audio_or_consume_new_intents_during_pause() {
    let mut host = AudioHost::default();
    let mut app = App::new(frame());
    app.frame = None;
    app.master_gain = 0.;
    submit(&mut host, &mut app, true);
    assert_eq!(host.report().state, AudioState::Idle);
    assert_eq!(host.report().master_gain, 0.);
    assert_eq!(host.report().generations_started, 0);
    let mut pending = frame();
    pending.paused = true;
    let voice = &mut Arc::make_mut(&mut pending.voices)[0];
    voice.generation = 1;
    voice.playing = true;
    app.frame = Some(pending);
    app.master_gain = 0.25;
    submit(&mut host, &mut app, true);
    assert_eq!(host.report().pending_voices, 1);
    assert_eq!(host.report().master_gain, 0.25);
    let reads = app.audio_reads;
    app.voice().generation = 2;
    app.master_gain = 0.5;
    host.pause(&mut app);
    assert_eq!(
        app.audio_reads, reads,
        "visibility pause only reuses accepted audio intents"
    );
    assert_eq!(host.report().voices[0].generation, 1);
    assert_eq!(host.report().master_gain, 0.5);
    assert_eq!(host.report().pending_voices, 1);
    submit(&mut host, &mut app, false);
    assert_eq!(host.report().voices[0].generation, 2);
    assert!(host.last_frame.as_ref().unwrap().paused);
    app.frame.as_mut().unwrap().world = WorldId::new();
    app.master_gain = 0.1;
    submit(&mut host, &mut app, true);
    assert_eq!(host.report().master_gain, 0.1);
    assert_eq!(host.report().state, AudioState::Idle);
    assert_eq!(host.report().pending_voices, 1);
    assert_eq!(host.report().generations_started, 0);
    assert!(app.errors.is_empty());
    host.shutdown();
    assert_eq!(host.report().master_gain, 0.1);
}

#[test]
fn invalid_host_volume_keeps_the_world_valid_and_content_failures_keep_accepted_volume() {
    let mut host = AudioHost::default();
    let mut app = App::new(frame());
    app.frame.as_mut().unwrap().paused = true;
    app.voice().generation = 1;
    app.voice().playing = true;
    app.master_gain = 0.4;
    submit(&mut host, &mut app, true);
    let world = host.report().world;
    for invalid in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        app.master_gain = invalid;
        submit(&mut host, &mut app, true);
    }
    assert_eq!(
        app.errors.len(),
        1,
        "repeated invalid controls report one bounded error"
    );
    assert!(app.errors[0].contains("主音量"));
    assert_eq!(host.report().state, AudioState::Idle);
    assert!(host.report().error.is_none());
    assert_eq!(host.report().world, world);
    assert_eq!(host.report().master_gain, 0.4);
    assert_eq!(host.report().pending_voices, 1);
    app.master_gain = 0.2;
    submit(&mut host, &mut app, true);
    assert_eq!(host.report().master_gain, 0.2);
    assert_eq!(app.errors.len(), 1);
    app.master_gain = 0.3;
    app.failure = Some("position query failed".into());
    submit(&mut host, &mut app, true);
    assert_eq!(host.report().state, AudioState::Unavailable);
    assert_eq!(host.report().master_gain, 0.3);
    assert_eq!(app.errors.len(), 2);
    app.failure = None;
    app.master_gain = 0.6;
    app.frame = None;
    submit(&mut host, &mut app, true);
    assert_eq!(host.report().master_gain, 0.6);
    assert_eq!(host.report().generations_started, 0);
    assert_eq!(host.report().pending_voices, 0);
    host.shutdown();
}
fn frame() -> AudioFrame {
    AudioFrame {
        world: WorldId::new(),
        paused: false,
        listener: None,
        voices: Arc::from([AudioVoice {
            entity: EntityId::new(),
            generation: 0,
            clip: AssetId::new(),
            data: Arc::new(AudioData {
                version: AUDIO_VERSION,
                sample_rate: 48_000,
                channels: 1,
                samples: Arc::from([0, 8192, -8192, 0]),
            }),
            playing: false,
            looping: false,
            gain: 1.,
            spatial: None,
        }]),
    }
}

fn submit(host: &mut AudioHost, app: &mut App, active: bool) {
    let world = app
        .frame
        .as_ref()
        .map(|frame| frame.world)
        .or(host.scope)
        .unwrap_or_default();
    host.submit(app, world, Camera::default(), active);
}

#[test]
fn inactive_and_runtime_paused_frames_hold_new_generations_without_opening_a_device() {
    let mut host = AudioHost::default();
    let mut app = App::new(frame());
    app.voice().playing = true;
    app.voice().generation = 1;
    submit(&mut host, &mut app, false);
    let report = host.report();
    assert_eq!(report.state, AudioState::Idle);
    assert_eq!(report.pending_voices, 1);
    assert_eq!(report.voices[0].generation, 1);
    assert_eq!(report.generations_started, 0);
    let reads = app.audio_reads;
    host.wait_for_progress(std::time::Duration::from_millis(500));
    assert_eq!(app.audio_reads, reads);
    assert_eq!(host.report().pending_voices, 1);
    assert_eq!(host.report().generations_started, 0);
    assert!(host.last_frame.as_ref().unwrap().paused);
    // The host pauses its owned frame, preserving the application's accepted intent and batch.
    assert!(!app.frame.as_ref().unwrap().paused);
    assert!(Arc::ptr_eq(
        &host.last_frame.as_ref().unwrap().voices,
        &app.frame.as_ref().unwrap().voices
    ));

    app.voice().generation = 2;
    app.frame.as_mut().unwrap().paused = true;
    submit(&mut host, &mut app, true);
    submit(&mut host, &mut app, true);
    let report = host.report();
    assert_eq!(report.state, AudioState::Idle);
    assert_eq!(report.pending_voices, 1);
    assert_eq!(report.voices[0].generation, 2);
    assert_eq!(report.voices[0].position_seconds, 0.);
    assert_eq!(report.generations_started, 0);
    assert!(app.errors.is_empty());
    host.shutdown();
    assert!(host.last_frame.is_none());
    assert!(host.report().world.is_none());
    assert_eq!(host.report().pending_voices, 0);
    assert_eq!(host.report().generations_started, 0);
}

#[test]
fn visibility_pause_reuses_the_accepted_frame_without_a_redraw_or_replaying_intents() {
    let mut host = AudioHost::default();
    let mut app = App::new(frame());
    submit(&mut host, &mut app, true);
    assert_eq!(app.audio_reads, 1);
    // A visibility event cannot consume a newer application intent before its next accepted view.
    app.voice().generation = 1;
    app.voice().playing = true;
    host.pause(&mut app);
    assert_eq!(app.audio_reads, 1);
    assert_eq!(host.report().voices[0].generation, 0);
    assert_eq!(host.report().pending_voices, 0);
    assert!(host.last_frame.as_ref().unwrap().paused);

    submit(&mut host, &mut app, false);
    assert_eq!(app.audio_reads, 2);
    assert_eq!(host.report().pending_voices, 1);
    for _ in 0..3 {
        host.pause(&mut app);
    }
    assert_eq!(app.audio_reads, 2);
    let report = host.report();
    assert_eq!(report.state, AudioState::Idle);
    assert_eq!(report.voices[0].generation, 1);
    assert_eq!(report.voices[0].position_seconds, 0.);
    assert_eq!(report.generations_started, 0);
    assert_eq!(report.pending_voices, 1);
    host.shutdown();
    host.pause(&mut app);
    assert_eq!(app.audio_reads, 2);
    assert_eq!(host.report().pending_voices, 0);
    assert!(app.errors.is_empty());
}

#[test]
fn failed_frames_are_reported_once_per_scope_and_none_retires_the_previous_scope() {
    let mut host = AudioHost::default();
    let mut app = App::new(frame());
    app.voice().gain = f32::NAN; // Fails full-frame validation before any device initialization.
    submit(&mut host, &mut app, true);
    submit(&mut host, &mut app, true);
    host.pause(&mut app);
    assert_eq!(app.errors.len(), 1);
    assert_eq!(host.report().state, AudioState::Unavailable);
    assert_eq!(host.report().generations_started, 0);
    app.frame.as_mut().unwrap().world = WorldId::new();
    submit(&mut host, &mut app, true);
    assert_eq!(app.errors.len(), 2);
    assert_eq!(app.errors[0], app.errors[1]);
    let failed = app.frame.take().unwrap();
    let error = host.report().error;
    submit(&mut host, &mut app, true);
    assert!(host.last_frame.is_none());
    assert!(host.last_error.is_none());
    assert!(host.report().world.is_none());
    assert_eq!(host.report().error, error);
    let reads = app.audio_reads;
    host.pause(&mut app);
    assert_eq!(app.audio_reads, reads);
    assert_eq!(app.errors.len(), 2);
    app.frame = Some(failed);
    submit(&mut host, &mut app, true);
    assert_eq!(app.errors.len(), 3);

    app.frame = Some(frame());
    app.frame.as_mut().unwrap().paused = true;
    app.voice().generation = 1;
    app.voice().playing = true;
    submit(&mut host, &mut app, true);
    assert_eq!(app.errors.len(), 3);
    assert!(host.last_error.is_none());
    assert_eq!(host.report().state, AudioState::Idle);
    assert_eq!(host.report().pending_voices, 1);
    assert!(host.report().error.is_none());
    host.shutdown();
}

#[test]
fn spatial_intents_use_the_accepted_camera_and_do_not_open_a_device_while_paused() -> Result<()> {
    use comet_audio::SpatialVoice;
    use comet_core::math::Vec3;
    let mut host = AudioHost::default();
    let mut app = App::new(frame());
    app.frame.as_mut().unwrap().paused = true;
    app.voice().playing = true;
    app.voice().generation = 1;
    app.voice().spatial = Some(SpatialVoice {
        position: [1., 0., -2.],
        min_distance: 1.,
        max_distance: 30.,
    });
    let world = app.frame.as_ref().unwrap().world;
    let camera = Camera {
        eye: Vec3::new(3., 2., 4.),
        target: Vec3::new(-1., 1., 0.),
    };
    host.submit(&mut app, world, camera, true);
    let pose = host
        .last_frame
        .as_ref()
        .context("accepted spatial frame")?
        .listener
        .context("camera listener")?;
    assert_eq!(pose.position, camera.eye.to_array());
    let rotation = Quat::from_array(pose.rotation);
    assert!((rotation * Vec3::NEG_Z - (camera.target - camera.eye).normalize()).length() < 1e-6);
    assert_eq!(host.report().state, AudioState::Idle);
    assert_eq!(host.report().generations_started, 0);
    assert_eq!(host.report().pending_voices, 1);
    assert!(app.errors.is_empty());
    // A different requested generation cannot be consumed during the visibility callback.
    app.voice().generation = 2;
    host.pause(&mut app);
    assert_eq!(app.audio_reads, 1);
    assert_eq!(host.report().voices[0].generation, 1);
    assert_eq!(host.last_frame.as_ref().unwrap().listener, Some(pose));
    host.shutdown();
    Ok(())
}

#[test]
fn stale_worlds_and_failed_position_queries_are_rejected_before_device_work() {
    let mut host = AudioHost::default();
    let mut app = App::new(frame());
    app.voice().playing = true;
    app.voice().generation = 1;
    let view_world = WorldId::new();
    for _ in 0..2 {
        host.submit(&mut app, view_world, Camera::default(), true);
    }
    assert_eq!(app.errors.len(), 1);
    assert!(app.errors[0].contains("不属于当前视图世界"));
    assert_eq!(host.report().state, AudioState::Unavailable);
    assert_eq!(host.report().generations_started, 0);
    assert!(host.last_frame.is_none());
    app.failure = Some("世界位置查询失败。".repeat(1000));
    for _ in 0..2 {
        host.submit(&mut app, view_world, Camera::default(), true);
    }
    assert_eq!(app.errors.len(), 2);
    assert!(app.errors[1].len() <= 4096);
    assert!(app.errors[1].contains("世界位置查询失败。"));
    assert_eq!(host.report().active_voices, 0);
    assert_eq!(host.report().generations_started, 0);
}

#[test]
fn camera_listener_uses_negative_z_forward_and_rejects_degenerate_views() -> Result<()> {
    use comet_core::math::Vec3;
    let front = listener(Camera {
        eye: Vec3::Z,
        target: Vec3::ZERO,
    })?;
    assert!((Quat::from_array(front.rotation) * Vec3::NEG_Z - Vec3::NEG_Z).length() < 1e-6);
    let side = listener(Camera {
        eye: Vec3::X,
        target: Vec3::ZERO,
    })?;
    assert!((Quat::from_array(side.rotation) * Vec3::NEG_Z - Vec3::NEG_X).length() < 1e-6);
    for camera in [
        Camera {
            eye: Vec3::ZERO,
            target: Vec3::ZERO,
        },
        Camera {
            eye: Vec3::Y,
            target: Vec3::ZERO,
        },
        Camera {
            eye: Vec3::splat(f32::NAN),
            ..Default::default()
        },
    ] {
        assert!(listener(camera).is_err());
    }
    Ok(())
}
