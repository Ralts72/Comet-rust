use super::*;
use std::time::Duration;

#[test]
fn long_active_frames_are_forwarded_without_silent_clamping() {
    let start = Instant::now();
    let mut timing = FrameTiming::default();
    assert_eq!(timing.sample(start), 0.);
    assert_eq!(timing.sample(start + Duration::from_secs(2)), 2.);
}

#[test]
fn focus_visibility_and_platform_resume_never_catch_up_suspended_time() {
    let start = Instant::now();
    let mut timing = FrameTiming::default();
    timing.sample(start);
    assert!(timing.take_activity_change());
    assert!(!timing.take_activity_change());
    timing.focused(false);
    timing.focused(true);
    assert!(timing.active());
    assert!(timing.take_activity_change());
    assert_eq!(timing.sample(start + Duration::from_secs(100)), 0.);
    assert_eq!(timing.sample(start + Duration::from_millis(100_010)), 0.01);
    for set in [
        FrameTiming::occluded,
        FrameTiming::minimized,
        FrameTiming::suspended,
    ] {
        set(&mut timing, true);
        assert!(!timing.active());
        assert_eq!(timing.sample(start + Duration::from_secs(200)), 0.);
        set(&mut timing, false);
        assert!(timing.active());
        assert_eq!(timing.sample(start + Duration::from_secs(300)), 0.);
    }
}
