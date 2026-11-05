use super::*;
use comet_core::math::Quat;

fn spatial_frame(position: [f32; 3]) -> AudioFrame {
    let mut frame = frame(data(1, 48_000, 4800));
    frame.listener = Some(ListenerPose {
        position: [0.; 3],
        rotation: [0., 0., 0., 1.],
    });
    edit(&mut frame, |voice| {
        voice.looping = true;
        voice.spatial = Some(SpatialVoice {
            position,
            min_distance: 1.,
            max_distance: 30.,
        });
    });
    frame
}
fn place(frame: &mut AudioFrame, position: [f32; 3]) {
    edit(frame, |voice| {
        voice.spatial.as_mut().unwrap().position = position
    });
}
fn channels(pcm: &[f32]) -> [f32; 2] {
    let mut result = [0.; 2];
    for pair in pcm.as_chunks::<2>().0 {
        result[0] += pair[0].abs();
        result[1] += pair[1].abs();
    }
    result.map(|sum| sum / (pcm.len() / 2) as f32)
}
fn stable(output: &mut OutputState<CaptureBackend>) -> Vec<f32> {
    render(output, 512);
    render(output, 512)
}

#[test]
fn source_listener_motion_and_rotation_change_real_pcm_without_replay_or_conversion() -> Result<()>
{
    let mut output = output();
    let mut frame = spatial_frame([-3., 0., 0.]);
    output.submit(Some(&frame))?;
    let native = output.clips[&frame.voices[0].clip].sound.frames.clone();
    let left = channels(&stable(&mut output));
    assert!(left[0] > left[1] * 8. && left[0] > 0.05);
    let position = output.report().voices[0].position_seconds;
    place(&mut frame, [3., 0., 0.]);
    output.submit(Some(&frame))?;
    let right = channels(&stable(&mut output));
    assert!(right[1] > right[0] * 8. && right[1] > 0.05);
    assert!(output.report().voices[0].position_seconds > position);

    frame.listener.as_mut().unwrap().rotation =
        Quat::from_rotation_y(std::f32::consts::PI).to_array();
    output.submit(Some(&frame))?;
    let turned = channels(&stable(&mut output));
    assert!(turned[0] > turned[1] * 8.);
    frame.listener = Some(ListenerPose {
        position: [6., 0., 0.],
        rotation: [0., 0., 0., 1.],
    });
    output.submit(Some(&frame))?;
    let listener_moved = channels(&stable(&mut output));
    assert!(listener_moved[0] > listener_moved[1] * 8.);
    let report = output.report();
    assert_eq!(report.voices[0].relative_position, Some([-3., 0., 0.]));
    assert_eq!(report.listener, frame.listener);
    assert_eq!(report.generations_started, 1);
    assert!(Arc::ptr_eq(
        &native,
        &output.clips[&frame.voices[0].clip].sound.frames
    ));
    let track = output.voices[&frame.voices[0].entity]
        .track
        .as_ref()
        .unwrap();
    assert_eq!(track.sound_capacity(), 2);
    assert_eq!(track.sub_track_capacity(), 0);
    Ok(())
}

#[test]
fn fixed_distance_curve_interpolates_decibels_and_returns_from_silence_without_restart()
-> Result<()> {
    let mut output = output();
    let mut frame = spatial_frame([0., 0., -10.]);
    edit(&mut frame, |voice| {
        let spatial = voice.spatial.as_mut().unwrap();
        spatial.min_distance = 10.;
        spatial.max_distance = 30.;
    });
    output.submit(Some(&frame))?;
    let near = energy(&stable(&mut output));
    place(&mut frame, [0., 0., -20.]);
    output.submit(Some(&frame))?;
    let middle = energy(&stable(&mut output));
    // -30dB at the midpoint, apart from the SDK's small distance-dependent ear geometry.
    assert!((middle / near - 10_f32.powf(-30. / 20.)).abs() < 0.001);
    place(&mut frame, [0., 0., -30.]);
    output.submit(Some(&frame))?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    place(&mut frame, [0., 0., -10.]);
    output.submit(Some(&frame))?;
    assert!((energy(&stable(&mut output)) - near).abs() < 1e-5);
    assert_eq!(output.report().generations_started, 1);
    Ok(())
}

#[test]
fn private_rebase_preserves_ears_under_large_world_translations_and_equal_finite_extremes()
-> Result<()> {
    for (relative, offset) in [([2., 0., -2.], [16_777_216.; 3]), ([0.; 3], [f32::MAX; 3])] {
        let mut origin = output();
        let original = spatial_frame(relative);
        origin.submit(Some(&original))?;
        let expected = stable(&mut origin);
        assert!(energy(&expected) > 0.001);
        let mut rebased = output();
        let mut translated = original.clone();
        translated.listener.as_mut().unwrap().position = offset;
        place(
            &mut translated,
            std::array::from_fn(|i| offset[i] + relative[i]),
        );
        rebased.submit(Some(&translated))?;
        let actual = stable(&mut rebased);
        assert_eq!(actual, expected);
        let report = rebased.report();
        assert_eq!(report.listener.unwrap().position, offset);
        assert_eq!(report.voices[0].relative_position, Some(relative));
    }
    Ok(())
}

#[test]
fn spatial_stereo_folds_to_mono_and_mode_or_distance_changes_require_a_new_generation() -> Result<()>
{
    let mut output = output();
    let mut frame = spatial_frame([0., 0., -3.]);
    edit(&mut frame, |voice| voice.data = data(2, 48_000, 4800));
    output.submit(Some(&frame))?;
    let native = output.clips[&frame.voices[0].clip].sound.frames.clone();
    let pcm = stable(&mut output);
    assert!(
        pcm.as_chunks::<2>()
            .0
            .iter()
            .all(|pair| pair[0] < -0.01 && pair[1] < -0.01)
    );
    edit(&mut frame, |voice| {
        voice.spatial.as_mut().unwrap().max_distance = 40.
    });
    output.submit(Some(&frame))?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    assert_eq!(output.report().generations_started, 1);
    assert!(output.listener.is_none());
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    assert!(energy(&stable(&mut output)) > 0.01);
    edit(&mut frame, |voice| voice.spatial = None);
    output.submit(Some(&frame))?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    let stereo = stable(&mut output);
    assert!(
        stereo
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| pair[0] > 0.1 && pair[1] < -0.2)
    );
    assert_eq!(output.report().generations_started, 3);
    assert!(Arc::ptr_eq(
        &native,
        &output.clips[&frame.voices[0].clip].sound.frames
    ));
    Ok(())
}

#[test]
fn spatial_pause_new_generation_and_world_retirement_keep_real_output_silent_until_resume()
-> Result<()> {
    let mut output = output();
    let mut frame = spatial_frame([-3., 0., 0.]);
    output.submit(Some(&frame))?;
    assert!(energy(&stable(&mut output)) > 0.05);
    frame.paused = true;
    output.submit(Some(&frame))?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    let position = output.report().voices[0].position_seconds;
    place(&mut frame, [3., 0., 0.]);
    frame.listener.as_mut().unwrap().position = [0., 0., 1.];
    output.submit(Some(&frame))?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    assert_eq!(output.report().voices[0].position_seconds, position);
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    assert_eq!(output.report().pending_voices, 1);
    assert_eq!(energy(&stable(&mut output)), 0.);
    frame.world = WorldId::new();
    output.submit(Some(&frame))?;
    assert!(output.listener.is_none());
    assert!(output.voices.values().all(|voice| voice.track.is_none()));
    assert_eq!(energy(&stable(&mut output)), 0.);
    assert_eq!(output.report().generations_started, 1);
    frame.paused = false;
    output.submit(Some(&frame))?;
    let resumed = channels(&stable(&mut output));
    assert!(resumed[1] > resumed[0] * 5.);
    assert_eq!(output.report().generations_started, 2);
    let last = output.report();
    output.submit(None)?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    assert!(output.listener.is_none());
    assert_eq!(output.report().listener, last.listener);
    assert_eq!(output.report().voices[0].spatial, last.voices[0].spatial);
    Ok(())
}

#[test]
fn missing_listener_bad_quaternion_and_unsafe_finite_positions_fail_before_any_voice_starts() {
    let good = spatial_frame([1., 0., -1.]);
    let mut cases = Vec::new();
    let mut missing = good.clone();
    missing.listener = None;
    cases.push(missing);
    for rotation in [[0.; 4], [0., 0., 0., 2.], [f32::NAN, 0., 0., 1.]] {
        let mut bad = good.clone();
        bad.listener.as_mut().unwrap().rotation = rotation;
        cases.push(bad);
    }
    for position in [[f32::MAX, 0., 0.], [1e20, 0., 0.], [f32::INFINITY, 0., 0.]] {
        let mut bad = good.clone();
        place(&mut bad, position);
        cases.push(bad);
    }
    let mut extreme = good.clone();
    extreme.listener.as_mut().unwrap().position = [-f32::MAX; 3];
    place(&mut extreme, [f32::MAX; 3]);
    cases.push(extreme);
    for (min_distance, max_distance) in [(-1., 1.), (1., 1.), (2., 1.), (0., f32::from_bits(1))] {
        let mut bad = good.clone();
        edit(&mut bad, |voice| {
            let spatial = voice.spatial.as_mut().unwrap();
            spatial.min_distance = min_distance;
            spatial.max_distance = max_distance;
        });
        cases.push(bad);
    }
    for bad in cases {
        let mut output = output();
        assert!(output.submit(Some(&bad)).is_err());
        assert!(output.manager.is_none());
        assert_eq!(output.report().generations_started, 0);
        assert_eq!(output.report().state, AudioState::Unavailable);
        assert_eq!(output.report().world, Some(bad.world));
    }
}

#[test]
fn admitted_large_relative_distance_produces_finite_pcm_and_host_reject_stops_existing_routes()
-> Result<()> {
    let mut output = output();
    let mut frame = spatial_frame([1e18, 0., 0.]);
    edit(&mut frame, |voice| {
        let spatial = voice.spatial.as_mut().unwrap();
        spatial.min_distance = 0.;
        spatial.max_distance = 2e18;
    });
    output.submit(Some(&frame))?;
    assert!(energy(&stable(&mut output)) > 0.001);
    let position = output.report().voices[0].position_seconds;
    let rejected = WorldId::new();
    output.reject(Some(rejected), "声音准备错误🙂".repeat(1000));
    let report = output.report();
    assert_eq!(report.world, Some(rejected));
    assert_eq!(report.state, AudioState::Unavailable);
    assert!(report.error.as_ref().unwrap().len() <= 4096);
    assert_eq!(report.active_voices, 0);
    assert_eq!(report.voices[0].position_seconds, position);
    assert!(output.manager.is_none() && output.listener.is_none());
    assert!(output.voices.values().all(|voice| voice.track.is_none()));
    frame.world = rejected;
    assert!(output.submit(Some(&frame)).is_err());
    assert_eq!(output.report().generations_started, 1);
    frame.world = WorldId::new();
    output.submit(Some(&frame))?;
    assert!(energy(&stable(&mut output)) > 0.001);
    assert_eq!(output.report().generations_started, 2);
    Ok(())
}

#[test]
fn natural_spatial_completion_and_initial_host_reject_never_trigger_implicit_replay() -> Result<()>
{
    let mut output = output();
    let mut frame = spatial_frame([-1., 0., 0.]);
    edit(&mut frame, |voice| {
        voice.looping = false;
        voice.data = data(1, 48_000, 480);
    });
    output.submit(Some(&frame))?;
    render(&mut output, 1024);
    place(&mut frame, [1., 0., 0.]);
    output.submit(Some(&frame))?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    assert_eq!(output.report().generations_started, 1);
    assert!(output.voices[&frame.voices[0].entity].track.is_some());
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    assert!(energy(&render(&mut output, 256)) > 0.01);
    assert_eq!(output.report().generations_started, 2);

    let mut rejected = Output::default();
    let world = WorldId::new();
    rejected.reject(world, "host camera rejected");
    assert_eq!(rejected.report().state, AudioState::Unavailable);
    assert_eq!(rejected.report().world, Some(world));
    assert_eq!(rejected.report().generations_started, 0);
    rejected.shutdown();
    assert_eq!(
        rejected.report().error.as_deref(),
        Some("host camera rejected")
    );
    Ok(())
}

#[test]
fn all_spatial_voices_fit_an_unprocessed_world_handoff_and_old_tracks_retire() -> Result<()> {
    let mut output = output();
    let mut frame = spatial_frame([-3., 0., 0.]);
    let mut shared = frame.voices[0].clone();
    shared.gain = 1. / MAX_VOICES as f32;
    frame.voices = (0..MAX_VOICES)
        .map(|_| AudioVoice {
            entity: EntityId::new(),
            ..shared.clone()
        })
        .collect();
    output.submit(Some(&frame))?;
    let manager = output.manager.as_ref().unwrap();
    assert_eq!(manager.sub_track_capacity(), MAX_VOICES * 2);
    assert_eq!(manager.num_sub_tracks(), MAX_VOICES);
    assert_eq!(output.clips.len(), 1);
    // No Renderer callback has retired the old world yet: both complete track sets must fit.
    frame.world = WorldId::new();
    for voice in Arc::make_mut(&mut frame.voices) {
        voice.spatial.as_mut().unwrap().position = [3., 0., 0.];
    }
    output.submit(Some(&frame))?;
    assert_eq!(
        output.manager.as_ref().unwrap().num_sub_tracks(),
        MAX_VOICES * 2
    );
    let pcm = channels(&stable(&mut output));
    assert!(pcm[1] > pcm[0] * 8. && pcm[1] > 0.05);
    assert_eq!(
        output.manager.as_ref().unwrap().num_sub_tracks(),
        MAX_VOICES
    );
    assert_eq!(output.report().generations_started, (MAX_VOICES * 2) as u64);
    assert_eq!(output.clips.len(), 1);
    output.submit(None)?;
    assert_eq!(energy(&stable(&mut output)), 0.);
    assert_eq!(output.manager.as_ref().unwrap().num_sub_tracks(), 0);
    assert!(output.listener.is_none());
    frame.world = WorldId::new();
    output.submit(Some(&frame))?;
    assert!(energy(&stable(&mut output)) > 0.05);
    Ok(())
}
