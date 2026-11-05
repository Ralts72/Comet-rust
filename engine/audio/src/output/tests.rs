use super::*;
use comet_asset_data::audio::AUDIO_VERSION;
use kira::backend::{Renderer, mock::MockBackend};

mod master_gain;
mod spatial;

// Kira's official MockBackend exposes control/position but not its PCM buffer. This test-only
// backend retains its public Renderer to inspect the actual mixer output, without an audio device.
#[derive(Default)]
struct CaptureBackend {
    renderer: Option<Renderer>,
    error: Option<String>,
}
impl Backend for CaptureBackend {
    type Settings = ();
    type Error = ();
    fn setup(_: (), _: usize) -> std::result::Result<(Self, u32), ()> {
        Ok((Self::default(), 48_000))
    }
    fn start(&mut self, renderer: Renderer) -> std::result::Result<(), ()> {
        self.renderer = Some(renderer);
        Ok(())
    }
}
fn output() -> OutputState<CaptureBackend> {
    OutputState::new(|backend| backend.error.take())
}
fn render(output: &mut OutputState<CaptureBackend>, frames: usize) -> Vec<f32> {
    let mut pcm = vec![0.; frames * 2];
    if let Some(manager) = &mut output.manager {
        let renderer = manager.backend_mut().renderer.as_mut().unwrap();
        for chunk in pcm.chunks_mut(256) {
            renderer.on_start_processing();
            renderer.process(chunk, 2);
        }
        renderer.on_start_processing();
    }
    assert!(pcm.iter().all(|sample| sample.is_finite()));
    pcm
}
fn data(channels: u16, sample_rate: u32, frames: usize) -> Arc<AudioData> {
    let samples = (0..frames)
        .flat_map(|_| {
            if channels == 1 {
                vec![8192]
            } else {
                vec![8192, -16384]
            }
        })
        .collect::<Vec<_>>();
    Arc::new(AudioData {
        version: AUDIO_VERSION,
        sample_rate,
        channels,
        samples: samples.into(),
    })
}
fn frame(data: Arc<AudioData>) -> AudioFrame {
    AudioFrame {
        world: WorldId::new(),
        paused: false,
        listener: None,
        voices: Arc::from([AudioVoice {
            entity: EntityId::new(),
            generation: 1,
            clip: AssetId::new(),
            data,
            playing: true,
            looping: false,
            gain: 1.,
            spatial: None,
        }]),
    }
}
fn edit(frame: &mut AudioFrame, edit: impl FnOnce(&mut AudioVoice)) {
    edit(&mut Arc::make_mut(&mut frame.voices)[0]);
}
fn energy(pcm: &[f32]) -> f32 {
    pcm.iter().map(|sample| sample.abs()).sum::<f32>() / pcm.len() as f32
}

#[test]
fn silent_construction_and_none_never_open_a_device() -> Result<()> {
    let mut output = Output::default();
    assert_eq!(output.report().state, AudioState::Idle);
    output.set_master_gain(0.)?;
    assert!(output.set_master_gain(f32::NAN).is_err());
    output.submit(None)?;
    output.stop();
    output.shutdown();
    assert_eq!(output.report().state, AudioState::Idle);
    assert_eq!(output.report().generations_started, 0);
    assert_eq!(output.report().master_gain, 0.);
    Ok(())
}

#[test]
fn renderer_preserves_pcm_channels_and_resamples_product_rates() -> Result<()> {
    for (channels, rate) in [(1, 8_000), (1, 192_000), (2, 44_100), (2, 48_000)] {
        let mut output = output();
        let frame = frame(data(channels, rate, (rate / 10) as usize));
        output.submit(Some(&frame))?;
        let pcm = render(&mut output, 2400);
        let tail = &pcm[512..];
        assert!(energy(tail) > 0.1);
        for pair in tail.as_chunks::<2>().0 {
            assert!(pair[0] > 0.1);
            if channels == 1 {
                assert!((pair[0] - pair[1]).abs() < 1e-6);
            } else {
                assert!(pair[1] < -0.2);
            }
        }
        let position = output.report().voices[0].position_seconds;
        assert!(
            (position - 0.05).abs() < 0.004,
            "rate={rate}, position={position}"
        );
    }
    Ok(())
}

#[test]
fn natural_completion_is_not_replayed_and_cache_identity_is_reused() -> Result<()> {
    let mut output = output();
    let frame = frame(data(1, 48_000, 480));
    output.submit(Some(&frame))?;
    let native = output.clips[&frame.voices[0].clip].sound.frames.clone();
    assert!(energy(&render(&mut output, 256)) > 0.01);
    for _ in 0..4 {
        output.submit(Some(&frame))?;
        render(&mut output, 256);
    }
    assert_eq!(output.report().generations_started, 1);
    assert!(!output.report().voices[0].playing);
    assert!(Arc::ptr_eq(
        &native,
        &output.clips[&frame.voices[0].clip].sound.frames
    ));
    output.submit(Some(&frame))?;
    assert_eq!(energy(&render(&mut output, 512)), 0.);
    assert_eq!(output.report().generations_started, 1);
    Ok(())
}

#[test]
fn pause_freezes_progress_and_new_generation_waits_for_resume() -> Result<()> {
    let mut output = output();
    let mut frame = frame(data(1, 48_000, 4800));
    edit(&mut frame, |voice| voice.looping = true);
    output.submit(Some(&frame))?;
    render(&mut output, 512);
    frame.paused = true;
    output.submit(Some(&frame))?;
    render(&mut output, 256);
    let position = output.report().voices[0].position_seconds;
    assert_eq!(energy(&render(&mut output, 1024)), 0.);
    assert_eq!(output.report().voices[0].position_seconds, position);
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    assert_eq!(output.report().generations_started, 1);
    assert_eq!(output.report().pending_voices, 1);
    assert_eq!(energy(&render(&mut output, 256)), 0.);
    frame.paused = false;
    output.submit(Some(&frame))?;
    assert!(energy(&render(&mut output, 512)) > 0.1);
    assert_eq!(output.report().generations_started, 2);
    assert_eq!(output.report().pending_voices, 0);
    Ok(())
}

#[test]
fn looping_gain_changes_stop_and_new_world_have_precise_generation_semantics() -> Result<()> {
    let mut output = output();
    let mut frame = frame(data(2, 48_000, 480));
    edit(&mut frame, |voice| voice.looping = true);
    output.submit(Some(&frame))?;
    let full = energy(&render(&mut output, 2000)[512..]);
    assert!(full > 0.1);
    assert!(output.report().voices[0].position_seconds < 0.011);
    edit(&mut frame, |voice| voice.gain = 0.25);
    output.submit(Some(&frame))?;
    let quiet = energy(&render(&mut output, 1024)[512..]);
    assert!((quiet / full - 0.25).abs() < 0.01);
    assert_eq!(output.report().generations_started, 1);
    edit(&mut frame, |voice| voice.gain = 0.);
    output.submit(Some(&frame))?;
    assert_eq!(energy(&render(&mut output, 512)[256..]), 0.);
    edit(&mut frame, |voice| voice.gain = 0.25);
    output.submit(Some(&frame))?;
    edit(&mut frame, |voice| voice.looping = false);
    output.submit(Some(&frame))?;
    render(&mut output, 1024);
    assert_eq!(energy(&render(&mut output, 512)), 0.);
    edit(&mut frame, |voice| {
        voice.generation += 1;
        voice.looping = true;
    });
    output.submit(Some(&frame))?;
    render(&mut output, 512);
    let position = output.report().voices[0].position_seconds;
    output.submit(None)?;
    assert!(output.report().world.is_none());
    assert_eq!(output.report().voices[0].position_seconds, position);
    assert_eq!(energy(&render(&mut output, 256)), 0.);
    output.submit(Some(&frame))?;
    assert_eq!(energy(&render(&mut output, 256)), 0.);
    assert_eq!(output.report().generations_started, 2);
    frame.world = WorldId::new();
    output.submit(Some(&frame))?;
    assert!(energy(&render(&mut output, 512)) > 0.01);
    assert_eq!(output.report().generations_started, 3);
    Ok(())
}

#[test]
fn accepted_pcm_replacement_requires_a_new_generation() -> Result<()> {
    let mut output = output();
    let mut frame = frame(data(1, 48_000, 4800));
    output.submit(Some(&frame))?;
    render(&mut output, 256);
    edit(&mut frame, |voice| voice.data = data(2, 48_000, 4800));
    output.submit(Some(&frame))?;
    assert_eq!(energy(&render(&mut output, 256)), 0.);
    assert_eq!(output.report().generations_started, 1);
    assert!(output.clips.is_empty());
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    let pcm = render(&mut output, 512);
    assert!(
        pcm[512..]
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| pair[0] > 0.1 && pair[1] < -0.2)
    );
    assert_eq!(output.report().generations_started, 2);
    Ok(())
}

#[test]
fn full_frame_validation_precedes_native_changes_and_failed_world_never_replays() -> Result<()> {
    let mut output = output();
    let mut frame = frame(data(1, 48_000, 4800));
    output.submit(Some(&frame))?;
    render(&mut output, 256);
    let count = output.report().generations_started;
    let mut bad = frame.clone();
    let mut voices = bad.voices.to_vec();
    voices[0].generation += 1;
    let mut invalid = voices[0].clone();
    invalid.entity = EntityId::new();
    invalid.gain = f32::NAN;
    voices.push(invalid);
    bad.voices = voices.into();
    assert!(output.submit(Some(&bad)).is_err());
    assert_eq!(output.report().generations_started, count);
    assert_eq!(output.report().state, AudioState::Unavailable);
    assert!(output.manager.is_none());
    assert!(output.submit(Some(&frame)).is_err());
    assert_eq!(output.report().generations_started, count);
    output.submit(None)?;
    assert_eq!(output.report().state, AudioState::Unavailable);
    frame.world = WorldId::new();
    output.submit(Some(&frame))?;
    assert_eq!(output.report().state, AudioState::Ready);
    assert_eq!(output.report().generations_started, count + 1);
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    edit(&mut frame, |voice| voice.generation -= 1);
    assert!(output.submit(Some(&frame)).is_err());
    Ok(())
}

#[test]
fn async_device_failure_is_bounded_and_preserves_last_real_observations() -> Result<()> {
    let mut output = output();
    let frame = frame(data(1, 48_000, 4800));
    output.submit(Some(&frame))?;
    render(&mut output, 512);
    let position = output.report().voices[0].position_seconds;
    output.manager.as_mut().unwrap().backend_mut().error = Some("设备错误🙂".repeat(1000));
    assert!(output.submit(Some(&frame)).is_err());
    let report = output.report();
    assert_eq!(report.state, AudioState::Unavailable);
    assert!(report.error.as_ref().unwrap().len() <= 4096);
    assert_eq!(report.voices[0].position_seconds, position);
    assert_eq!(report.generations_started, 1);
    output.stop();
    assert_eq!(output.report().error, report.error);
    assert_eq!(output.report().voices[0].position_seconds, position);
    assert_eq!(output.report().generations_started, 1);
    Ok(())
}

#[derive(Default)]
struct MissingDevice;
impl Backend for MissingDevice {
    type Settings = ();
    type Error = &'static str;
    fn setup(_: (), _: usize) -> std::result::Result<(Self, u32), Self::Error> {
        Err("no output device")
    }
    fn start(&mut self, _: Renderer) -> std::result::Result<(), Self::Error> {
        unreachable!()
    }
}
#[test]
fn unavailable_device_does_not_retry_same_world_and_paused_new_voice_opens_nothing() -> Result<()> {
    let mut output = OutputState::<MissingDevice>::new(|_| None);
    let mut frame = frame(data(1, 48_000, 480));
    frame.paused = true;
    output.submit(Some(&frame))?;
    assert_eq!(output.report().state, AudioState::Idle);
    assert_eq!(output.report().pending_voices, 1);
    frame.paused = false;
    assert!(output.submit(Some(&frame)).is_err());
    assert!(output.report().error.unwrap().contains("no output device"));
    assert_eq!(output.report().generations_started, 0);
    assert!(output.submit(Some(&frame)).is_err());
    output.stop();
    assert_eq!(output.report().state, AudioState::Unavailable);
    Ok(())
}

#[test]
fn official_mock_backend_exercises_the_same_handle_control_path() -> Result<()> {
    let mut output = OutputState::<MockBackend>::new(|_| None);
    let mut frame = frame(data(1, 48_000, 4800));
    edit(&mut frame, |voice| voice.looping = true);
    output.submit(Some(&frame))?;
    let backend = output.manager.as_mut().unwrap().backend_mut();
    backend.set_sample_rate(48_000);
    for _ in 0..4 {
        backend.on_start_processing();
        backend.process();
    }
    backend.on_start_processing();
    assert!(output.report().voices[0].position_seconds > 0.);
    frame.paused = true;
    output.submit(Some(&frame))?;
    let backend = output.manager.as_mut().unwrap().backend_mut();
    backend.on_start_processing();
    backend.process();
    backend.on_start_processing();
    assert_eq!(
        output.voices[&frame.voices[0].entity]
            .handle
            .as_ref()
            .unwrap()
            .state(),
        PlaybackState::Paused
    );
    output.stop();
    let backend = output.manager.as_mut().unwrap().backend_mut();
    backend.on_start_processing();
    backend.process();
    backend.on_start_processing();
    assert_eq!(
        output.voices[&frame.voices[0].entity]
            .handle
            .as_ref()
            .unwrap()
            .state(),
        PlaybackState::Stopped
    );
    Ok(())
}
