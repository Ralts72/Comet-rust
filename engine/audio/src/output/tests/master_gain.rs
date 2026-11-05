use super::*;

fn steady(output: &mut OutputState<CaptureBackend>) -> Vec<f32> {
    render(output, 512);
    render(output, 512)
}

#[test]
fn master_controls_real_2d_and_spatial_pcm_without_changing_source_gain_or_cache() -> Result<()> {
    for spatial in [false, true] {
        let mut output = output();
        let mut frame = frame(data(2, 48_000, 48_000));
        edit(&mut frame, |voice| {
            voice.looping = true;
            voice.gain = 0.5;
            if spatial {
                voice.spatial = Some(SpatialVoice {
                    position: [0., 0., -1.],
                    min_distance: 1.,
                    max_distance: 30.,
                });
            }
        });
        if spatial {
            frame.listener = Some(ListenerPose {
                position: [0.; 3],
                rotation: [0., 0., 0., 1.],
            });
        }
        output.submit(Some(&frame))?;
        let native = output.clips[&frame.voices[0].clip].sound.frames.clone();
        let full = energy(&steady(&mut output));
        assert!(full > 0.001);
        output.set_master_gain(0.25)?;
        let quiet = energy(&steady(&mut output));
        assert!((quiet / full - 0.25).abs() < 0.001, "spatial={spatial}");
        // Kira's main track consumes the newest target, not an accumulating volume FIFO.
        for gain in [0.1, 0.8, 0.4] {
            output.set_master_gain(gain)?;
        }
        assert!((energy(&steady(&mut output)) / full - 0.4).abs() < 0.001);
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1, 1.1] {
            assert!(output.set_master_gain(invalid).is_err());
        }
        assert_eq!(output.report().master_gain, 0.4);
        assert_eq!(output.report().state, AudioState::Ready);
        assert!(output.report().error.is_none());
        assert!((energy(&steady(&mut output)) / full - 0.4).abs() < 0.001);
        let position = output.report().voices[0].position_seconds;
        output.set_master_gain(0.)?;
        assert_eq!(energy(&steady(&mut output)), 0.);
        assert!(output.report().voices[0].position_seconds > position);
        output.set_master_gain(1.)?;
        assert!((energy(&steady(&mut output)) - full).abs() < 1e-5);
        assert_eq!(output.report().generations_started, 1);
        assert_eq!(output.voices[&frame.voices[0].entity].gain, 0.5);
        assert!(Arc::ptr_eq(
            &native,
            &output.clips[&frame.voices[0].clip].sound.frames
        ));
    }
    Ok(())
}

#[test]
fn initially_muted_sound_advances_and_finishes_without_an_unmuted_first_buffer_or_replay()
-> Result<()> {
    let mut output = output();
    output.set_master_gain(0.)?;
    assert!(output.manager.is_none());
    let frame = frame(data(1, 48_000, 480));
    output.submit(Some(&frame))?;
    let native = output.clips[&frame.voices[0].clip].sound.frames.clone();
    assert_eq!(energy(&render(&mut output, 256)), 0.);
    assert!(output.report().voices[0].position_seconds > 0.);
    assert_eq!(energy(&render(&mut output, 1024)), 0.);
    assert!(!output.report().voices[0].playing);
    output.set_master_gain(1.)?;
    output.submit(Some(&frame))?;
    assert_eq!(energy(&render(&mut output, 512)), 0.);
    assert_eq!(output.report().generations_started, 1);
    assert!(Arc::ptr_eq(
        &native,
        &output.clips[&frame.voices[0].clip].sound.frames
    ));
    Ok(())
}

#[test]
fn host_pause_keeps_new_generations_pending_while_master_changes_remain_independent() -> Result<()>
{
    let mut output = output();
    let mut frame = frame(data(1, 48_000, 48_000));
    edit(&mut frame, |voice| voice.looping = true);
    output.submit(Some(&frame))?;
    let native = output.clips[&frame.voices[0].clip].sound.frames.clone();
    steady(&mut output);
    frame.paused = true;
    output.submit(Some(&frame))?;
    steady(&mut output);
    let position = output.report().voices[0].position_seconds;
    output.set_master_gain(0.)?;
    assert_eq!(energy(&steady(&mut output)), 0.);
    assert_eq!(output.report().voices[0].position_seconds, position);
    edit(&mut frame, |voice| voice.generation += 1);
    output.submit(Some(&frame))?;
    output.set_master_gain(0.25)?;
    assert_eq!(energy(&steady(&mut output)), 0.);
    assert_eq!(output.report().pending_voices, 1);
    assert_eq!(output.report().generations_started, 1);
    frame.paused = false;
    output.submit(Some(&frame))?;
    assert!((energy(&steady(&mut output)) - 0.0625).abs() < 1e-5);
    assert_eq!(output.report().pending_voices, 0);
    assert_eq!(output.report().generations_started, 2);
    assert!(Arc::ptr_eq(
        &native,
        &output.clips[&frame.voices[0].clip].sound.frames
    ));
    Ok(())
}

#[test]
fn accepted_master_survives_retirement_bad_worlds_and_device_recreation() -> Result<()> {
    let mut output = output();
    output.set_master_gain(0.5)?;
    let mut frame = frame(data(1, 48_000, 48_000));
    frame.paused = true;
    output.submit(Some(&frame))?;
    assert_eq!(output.report().state, AudioState::Idle);
    assert_eq!(output.report().pending_voices, 1);
    assert!(output.manager.is_none());
    frame.paused = false;
    output.submit(Some(&frame))?;
    assert!((energy(&steady(&mut output)) - 0.125).abs() < 1e-5);
    output.submit(None)?;
    assert_eq!(output.report().master_gain, 0.5);
    frame.world = WorldId::new();
    output.submit(Some(&frame))?;
    assert!((energy(&steady(&mut output)) - 0.125).abs() < 1e-5);
    output.reject(Some(frame.world), "device stream failed");
    assert!(output.manager.is_none());
    output.set_master_gain(0.25)?;
    assert_eq!(output.report().state, AudioState::Unavailable);
    assert!(
        output.submit(Some(&frame)).is_err(),
        "changing gain cannot retry a failed world"
    );
    frame.world = WorldId::new();
    output.submit(Some(&frame))?;
    assert!((energy(&steady(&mut output)) - 0.0625).abs() < 1e-5);
    assert_eq!(output.report().state, AudioState::Ready);
    assert_eq!(output.report().master_gain, 0.25);
    assert_eq!(output.report().generations_started, 3);
    Ok(())
}
